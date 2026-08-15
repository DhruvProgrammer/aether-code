//! Top-level agent loop: Controller plans (with memory + repo context), Coder (Executor)
//! implements — with cost-aware model routing (§8) — then optional Reviewer/Tester
//! subagents run a structured handoff pass (spec §4, §7, §29).
//!
//! Wrapped by a *loop-engineering* outer loop: an explicit `EngineeringModel` is
//! maintained across plan→execute→verify→replan cycles. The multi-agent subsystem
//! (`crate::agents`) lets the SMALL LLM (controller) orchestrate specialized workers
//! (Explorer/Planner/Designer/Tester/Reviewer/Security Reviewer/…); the BIG LLM
//! (executor) implements via the `implementer` agent. The LoopEngine is the circuit
//! breaker that escalates or stops instead of blind-retrying.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use aether_mind::Mind;
use aether_models::ModelProvider;
use aether_permissions::Policy;
use aether_sessions::SessionStore;
use aether_tools::Tool;
use crate::agents::{
    AgentRegistry, AgentRouter, AgentStatus, LifecycleTracker, build_agent_context, run_agent,
};
use crate::eng::{LoopAction, LoopEngine, LoopState};
use crate::executor::Executor;
use crate::mode::Mode;
use crate::subagents::{run_role, SubagentResult, EXPLORER};
use crate::visual::{CorrectionExecutor, VisualReviewEngine, should_run_visual_review};
use aether_config::FrontendConfig;

const CODER_SYSTEM: &str = "You are the Coder (Executor). Implement the task using the available \
                            tools. When the work is complete, reply with a final summary and no tool calls.";

pub struct Agent {
    controller: Arc<dyn ModelProvider>,
    controller_model: String,
    executor_model: String,
    providers: HashMap<String, Arc<dyn ModelProvider>>,
    session: Option<Arc<SessionStore>>,
    session_id: String,
    mind: Option<Arc<Mind>>,
    embedder: Option<Arc<dyn ModelProvider>>,
    auto_extract: bool,
    memory_top_k: usize,
    cwd: PathBuf,
    policy: Policy,
    tools: HashMap<String, Arc<dyn Tool>>,
    subagents_enabled: bool,
    cheap_model: Option<String>,
    max_iterations: u32,
    context_max_tokens: u32,
    loop_budget: u32,
    /// LLM 3 — VISUAL FRONTEND REVIEWER (optional). When `Some`, the 3-LLM visual loop may run.
    reviewer: Option<Arc<dyn ModelProvider>>,
    reviewer_model: Option<String>,
    /// Frontend visual-engineering configuration (spec: 3-LLM visual review).
    frontend: FrontendConfig,
}

impl Agent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        controller: Arc<dyn ModelProvider>,
        controller_model: String,
        executor_model: String,
        providers: HashMap<String, Arc<dyn ModelProvider>>,
        session: Option<Arc<SessionStore>>,
        session_id: String,
        mind: Option<Arc<Mind>>,
        embedder: Option<Arc<dyn ModelProvider>>,
        auto_extract: bool,
        memory_top_k: usize,
        cwd: PathBuf,
        policy: Policy,
        tools: HashMap<String, Arc<dyn Tool>>,
        subagents_enabled: bool,
        cheap_model: Option<String>,
        max_iterations: u32,
        context_max_tokens: u32,
        loop_budget: u32,
        reviewer: Option<Arc<dyn ModelProvider>>,
        reviewer_model: Option<String>,
        frontend: FrontendConfig,
    ) -> Self {
        Self {
            controller,
            controller_model,
            executor_model,
            providers,
            session,
            session_id,
            mind,
            embedder,
            auto_extract,
            memory_top_k,
            cwd,
            policy,
            tools,
            subagents_enabled,
            cheap_model,
            max_iterations,
            context_max_tokens,
            loop_budget,
            reviewer,
            reviewer_model,
            frontend,
        }
    }

    /// Resolve an agent's `model` key ("controller" = SMALL LLM, "executor" = BIG LLM) to a
    /// provider + model string. This is the single enforcement point for two-LLM routing.
    fn resolve(&self, key: &str) -> (Arc<dyn ModelProvider>, String) {
        if key == "executor" {
            (
                self.providers
                    .get(&self.executor_model)
                    .cloned()
                    .unwrap_or_else(|| self.controller.clone()),
                self.executor_model.clone(),
            )
        } else {
            (self.controller.clone(), self.controller_model.clone())
        }
    }

    fn provider_for(&self, key: &str) -> Arc<dyn ModelProvider> {
        self.providers
            .get(key)
            .cloned()
            .unwrap_or_else(|| self.controller.clone())
    }

    pub async fn run(
        &self,
        task: &str,
        mode: Mode,
        existing_plan: Option<&str>,
        resume_session: Option<&str>,
    ) -> anyhow::Result<AgentOutcome> {
        // When resuming, all persistence (messages / kv / traces / run record) targets the
        // resumed session so the continuation is recorded under the same id.
        let sid = resume_session.unwrap_or(&self.session_id);
        if let Some(store) = &self.session {
            if let Err(e) = store.add_message(sid, "user", task) {
                eprintln!("aether: session persist failed (user message): {e}");
            }
        }

        // Agent subsystem: registry (TOML + builtins) and lifecycle tracker (depth/child limits).
        let registry = AgentRegistry::load_from_dir(&self.cwd);
        let mut lifecycle = LifecycleTracker::new(3, 5);

        // --- Loop engineering: establish the EngineeringModel -------------------
        let mut eng = LoopEngine::new(task);
        // Resume: seed the model from a prior session's persisted engineering state.
        if let (Some(rs), Some(store)) = (resume_session, &self.session) {
            if let Ok(Some(json)) = store.get_kv(rs, "engineering") {
                if let Ok(m) = serde_json::from_str::<crate::eng::EngineeringModel>(&json) {
                    eng.model = m;
                    println!("[RESUME] loaded engineering state from session {rs}");
                }
            }
        }
        eng.model.loop_state = LoopState::Understanding;
        println!("{}", eng.render_panel());

        // Assemble context: repo instructions + retrieved memory (spec §12, §9.7).
        let mut context = aether_mind::context::discover_context(&self.cwd);
        if let Some(mind) = &self.mind {
            if let Ok(mem) = mind.retrieve(task, self.embedder.as_deref(), self.memory_top_k).await {
                if !mem.is_empty() {
                    context.push_str(&format!("\n## Retrieved Memory\n{}", mem));
                }
            }
        }

        // PLAN MODE: read-only investigate -> plan. Never modify the project (spec §13-§21).
        if mode.is_plan() {
            return self.run_plan(task, &context).await;
        }

        // BUILD MODE may be asked to implement an existing plan; load + validate it (§22).
        let plan_context: String = if let Some(p) = existing_plan {
            format!(
                "{task}\n\n# Existing plan to implement\nValidate it is still fresh (files/requirements may have changed) before editing. Do not follow a stale plan blindly.\n{p}"
            )
        } else {
            task.to_string()
        };

        // Multi-agent: Explorer (SMALL LLM) gathers repo findings before the Controller plans.
        let mut exploration = String::new();
        if self.subagents_enabled {
            if let Some(explorer_def) = registry.find("explorer") {
                let mut run = lifecycle.start("explorer", None, sid, None, None);
                let (prov, model) = self.resolve(&explorer_def.model);
                let ctx = build_agent_context(
                    explorer_def,
                    &format!("Investigate the repository to plan this change:\n{task}"),
                    Mode::Build,
                    "",
                    "",
                );
                match run_agent(
                    explorer_def,
                    prov,
                    &model,
                    &self.tools,
                    &self.policy,
                    &self.cwd,
                    self.session.clone(),
                    sid,
                    &run,
                    &ctx,
                )
                .await
                {
                    Ok(r) => {
                        exploration = r.summary.clone();
                        println!("[EXPLORE] {}\n", r.summary);
                        if !r.findings.is_empty() {
                            for f in &r.findings {
                                eng.add_fact(f);
                            }
                        }
                persist_trace(&self.session, &self.session_id, "explore", "explorer", &r.summary);
                    }
                    Err(e) => eprintln!("explorer failed: {e}"),
                }
                lifecycle.finish(&mut run, AgentStatus::Completed);
            }
        }
        if !exploration.is_empty() {
            context.push_str(&format!("\n## Explorer findings\n{}", exploration));
        }

        // Verification plan (Tester + Reviewer, + Security Reviewer on risk/security). Its ids also
        // drive the engineering success criteria.
        let verify_ids: Vec<String> = if self.subagents_enabled {
            let high_risk = eng.model.risk_level == "high";
            AgentRouter::select_verification(&registry, task, high_risk)
        } else {
            Vec::new()
        };
        let mut criteria = vec!["reviewer passes".to_string(), "tester passes".to_string()];
        if verify_ids.iter().any(|i| i == "security-reviewer") {
            criteria.push("security passes".to_string());
        }
        eng.set_success_criteria(criteria);

        let loop_budget = self.loop_budget.max(1);
        let mut prev_result: Option<String> = None;
        let mut final_result = String::new();
        let mut last_review: Option<SubagentResult> = None;
        let mut last_test: Option<SubagentResult> = None;
        let mut escalation: Option<String> = None;

        // --- Closed loop: plan → execute → verify → (re)plan --------------------
        for iter in 0..loop_budget {
            eng.model.iteration = iter;
            let cycle_task: String = match &prev_result {
                None => plan_context.clone(),
                Some(prev) => format!(
                    "{plan_context}\n\n# Prior attempt result (adapt — do not repeat past failures)\n{prev}\n\n# Engineering state\n{}\n",
                    eng.state_summary()
                ),
            };

            // Plan (or replan) with the model-informed task.
            let plan = crate::controller::plan(
                self.controller.as_ref(),
                &self.controller_model,
                &cycle_task,
                &context,
                Mode::Build,
            )
            .await?;
            if let Some(store) = &self.session {
                if let Err(e) = store.add_message(sid, "assistant", &format!("[PLAN {}]\n{plan}", iter + 1)) {
                    eprintln!("aether: session persist failed (plan message): {e}");
                }
            }
            println!("[PLAN {}]\n{plan}\n", iter + 1);
            eng.set_strategy(&plan);
            eng.add_decision(&format!("plan iteration {}", iter + 1), "controller produced plan", 0.6);
            persist_trace(&self.session, sid, "plan", "controller", &plan.chars().take(240).collect::<String>());

            // Cost routing (§8): pick Coder model by task intent.
            let coder_model = crate::router::select_model(
                &cycle_task,
                self.cheap_model.as_deref(),
                &self.executor_model,
                &self.controller_model,
            );
            let coder = Executor::new(
                self.provider_for(&coder_model),
                coder_model,
                self.tools.clone(),
                self.policy.clone(),
                self.cwd.clone(),
                self.max_iterations,
                self.context_max_tokens,
                self.session.clone(),
                sid.to_string(),
                format!("{CODER_SYSTEM}\n{}", crate::mode::KARPATHY_POLICY),
                None,
            );
            let result = coder.run(&cycle_task).await?;
            eng.record_action(&format!("execute plan (iter {})", iter + 1));
            eng.observe("executor", &summarize(&result), None, None);
            persist_trace(&self.session, sid, "execute", "implementer", &summarize(&result));

            // Subagent handoff: the routed verification pipeline (spec §17-§18, §58).
            let mut review: Option<SubagentResult> = None;
            let mut test: Option<SubagentResult> = None;
            let mut security: Option<SubagentResult> = None;
            for aid in &verify_ids {
                if let Some(def) = registry.find(aid) {
                    let mut run = lifecycle.start(aid, None, sid, None, None);
                    let (prov, model) = self.resolve(&def.model);
                    let ctx = format!("Original task:\n{task}\n\nImplementation result:\n{result}");
                    match run_agent(
                        def,
                        prov,
                        &model,
                        &self.tools,
                        &self.policy,
                        &self.cwd,
                        self.session.clone(),
                        sid,
                        &run,
                        &ctx,
                    )
                    .await
                    {
                        Ok(r) => {
                            println!("[{}] {}\n", r.role.to_uppercase(), r.summary);
                            persist_trace(
                                &self.session,
                                sid,
                                "verify",
                                &aid,
                                &format!("{}: {}", r.status, r.summary.chars().take(240).collect::<String>()),
                            );
                            match aid.as_str() {
                                "tester" => test = Some(r.clone()),
                                "reviewer" => review = Some(r.clone()),
                                "security-reviewer" => security = Some(r.clone()),
                                _ => {}
                            }
                            eng.add_evidence(
                                &format!("{}: {}", r.role, r.summary),
                                &aid,
                                0.8,
                                None,
                                None,
                            );
                        }
                        Err(e) => eprintln!("{} failed: {e}", aid),
                    }
                    lifecycle.finish(&mut run, AgentStatus::Completed);
                }
            }
            last_review = review.clone();
            last_test = test.clone();

            // Update the EngineeringModel from verification evidence.
            if verify_ids.is_empty() {
                eng.mark_criteria_met("reviewer passes");
                eng.mark_criteria_met("tester passes");
            } else {
                if let Some(r) = &review {
                    if r.status == "ok" {
                        eng.mark_criteria_met("reviewer passes");
                    } else {
                        eng.note_failure(&format!("reviewer: {}", r.summary));
                    }
                }
                if let Some(t) = &test {
                    if t.status == "ok" {
                        eng.mark_criteria_met("tester passes");
                    } else {
                        eng.note_failure(&format!("tester: {}", t.summary));
                    }
                }
                if let Some(s) = &security {
                    if s.status == "ok" {
                        eng.mark_criteria_met("security passes");
                    } else {
                        eng.note_failure(&format!("security: {}", s.summary));
                    }
                }
            }

            if eng.detect_stagnation() {
                eng.set_next_best_action("STOP — approach is not converging; escalate to human");
            } else if !eng.model.unknowns.is_empty() {
                eng.set_next_best_action(&format!("Resolve open unknown: {}", eng.model.unknowns.last().unwrap()));
            } else {
                eng.set_next_best_action("Continue implementing remaining plan steps");
            }

            if let Some(store) = &self.session {
                if let Err(e) = store.set_kv(
                    sid,
                    "engineering",
                    &serde_json::to_string(&eng.model).unwrap_or_default(),
                ) {
                    eprintln!("aether: session persist failed (engineering kv): {e}");
                }
            }

            final_result = format!("{result}\n{}", self.handoff_text(&review, &test, &security));
            println!("{}", eng.render_panel());

            match eng.decide(iter + 1, loop_budget) {
                LoopAction::Escalate => {
                    persist_trace(&self.session, sid, "decision", "loop-engine", "ESCALATE");
                    escalation = Some(eng.escalation_briefing());
                    break;
                }
                LoopAction::Stop => {
                    persist_trace(&self.session, sid, "decision", "loop-engine", "STOP");
                    break;
                }
                LoopAction::Continue => {
                    persist_trace(&self.session, sid, "decision", "loop-engine", "CONTINUE");
                    prev_result = Some(final_result.clone());
                    continue;
                }
            }
        }

        if let Some(store) = &self.session {
            if let Err(e) = store.add_message(sid, "assistant", &final_result) {
                eprintln!("aether: session persist failed (final message): {e}");
            }
            if let Err(e) = store.record_run(
                sid,
                task,
                &eng.model.current_strategy.clone().unwrap_or_default(),
                &final_result,
            ) {
                eprintln!("aether: session persist failed (record_run): {e}");
            }
        }

        if self.auto_extract {
            if let (Some(mind), Some(prov)) = (&self.mind, &self.embedder) {
                let transcript = format!(
                    "task: {}\nplan: {}\nresult: {}",
                    task,
                    eng.model.current_strategy.clone().unwrap_or_default(),
                    final_result
                );
                let _ = aether_mind::extract::extract(mind, &transcript, prov.as_ref(), &self.controller_model).await;
            }
        }

        // --- Visual engineering loop (LLM 3): the 3-LLM frontend QA stage (spec §10) -------
        // Runs only for frontend tasks when LLM 3 (reviewer) is configured. LLM 1 (executor)
        // is mandatory and implements corrections; LLM 3 only critiques and approves.
        if !mode.is_plan() {
            if let Some(reviewer) = &self.reviewer {
                if should_run_visual_review(task, &self.reviewer_model, &self.frontend) {
                    println!("[VISUAL] entering 3-LLM visual engineering loop (FRONTEND_READY)");
                    let engine = VisualReviewEngine::new(
                        reviewer.clone(),
                        self.reviewer_model.clone().unwrap_or_default(),
                        self.controller.clone(),
                        self.controller_model.clone(),
                        self.frontend.clone(),
                        self.cwd.clone(),
                        self.session.clone(),
                        sid.to_string(),
                    );
                    let report = engine.run(task, &eng.model.current_strategy.clone().unwrap_or_default(), self).await;
                    println!("{}", report.summary);
                    final_result.push_str(&format!("\n\n## Visual Review\n{}\n", report.summary));
                    if let Some(store) = &self.session {
                        if let Err(e) = store.add_message(sid, "assistant", &format!("[VISUAL] {}", report.summary)) {
                            eprintln!("aether: session persist failed (visual message): {e}");
                        }
                    }
                }
            }
        }

        let mut outcome = AgentOutcome {
            plan: eng.model.current_strategy.clone().unwrap_or_default(),
            result: final_result,
            review: last_review,
            test: last_test,
            engineering: eng.state_summary(),
        };
        if let Some(esc) = &escalation {
            outcome.result.push_str(esc);
        }
        Ok(outcome)
    }

    /// PLAN MODE: investigate the repository (read-only) and produce a structured plan.
    /// Never modifies application source (spec §13-§21); Karpathy guidelines apply.
    async fn run_plan(&self, task: &str, context: &str) -> anyhow::Result<AgentOutcome> {
        let mut exploration = String::new();
        let p = self.controller.clone();
        match run_role(
            &EXPLORER,
            p,
            &self.controller_model,
            &self.tools,
            &self.policy,
            &self.cwd,
            self.session.clone(),
            &self.session_id,
            &format!("Investigate the repository to plan this change:\n{task}"),
        )
        .await
        {
            Ok(r) => {
                println!("[EXPLORE] {}\n", r.summary);
                exploration = r.summary.clone();
                persist_trace(&self.session, &self.session_id, "explore", "explorer", &r.summary);
            }
            Err(e) => eprintln!("explorer failed: {e}"),
        }

        let plan_input = format!("{context}\n\n# Repository Exploration\n{exploration}");
        let plan = crate::controller::plan(
            self.controller.as_ref(),
            &self.controller_model,
            task,
            &plan_input,
            Mode::Plan,
        )
        .await?;
        if let Some(store) = &self.session {
            let _ = store.add_message(&self.session_id, "assistant", &format!("[PLAN]\n{plan}"));
        }
        println!("[PLAN MODE]\n{plan}\n");
        Ok(AgentOutcome {
            plan: plan.clone(),
            result: plan,
            review: None,
            test: None,
            engineering: String::new(),
        })
    }

    fn handoff_text(
        &self,
        review: &Option<SubagentResult>,
        test: &Option<SubagentResult>,
        security: &Option<SubagentResult>,
    ) -> String {
        let mut s = String::new();
        if let Some(r) = review {
            s.push_str(&format!("\n## Reviewer ({})\n{}\n", r.status, r.summary));
            for f in &r.findings {
                s.push_str(&format!("- {f}\n"));
            }
        }
        if let Some(t) = test {
            s.push_str(&format!("\n## Tester ({})\n{}\n", t.status, t.summary));
            for f in &t.findings {
                s.push_str(&format!("- {f}\n"));
            }
        }
        if let Some(sec) = security {
            s.push_str(&format!("\n## Security Reviewer ({})\n{}\n", sec.status, sec.summary));
            for f in &sec.findings {
                s.push_str(&format!("- {f}\n"));
            }
        }
        s
    }
}

fn summarize(s: &str) -> String {
    s.lines().take(12).collect::<Vec<_>>().join("\n")
}

/// LLM 1 boundary for the visual loop: LLM 3's evidence flows to LLM 2 (correction plan),
/// then here to actually implement the correction via the BIG EXECUTOR. LLM 3 never calls this.
#[async_trait::async_trait(?Send)]
impl CorrectionExecutor for Agent {
    async fn implement_correction(&self, plan: &str) -> anyhow::Result<String> {
        let coder = Executor::new(
            self.provider_for(&self.executor_model),
            self.executor_model.clone(),
            self.tools.clone(),
            self.policy.clone(),
            self.cwd.clone(),
            self.max_iterations,
            self.context_max_tokens,
            self.session.clone(),
            self.session_id.clone(),
            format!("{CODER_SYSTEM}\n{}", crate::mode::KARPATHY_POLICY),
            None,
        );
        coder.run(plan).await
    }
}

/// Record a trace event for debugging/replay (spec Phase 6). No-op when session store is off.
/// Named `persist_trace` to avoid shadowing the `tracing` crate's macros.
fn persist_trace(store: &Option<Arc<SessionStore>>, session_id: &str, kind: &str, agent: &str, summary: &str) {
    if let Some(s) = store {
        let _ = s.record_trace(session_id, kind, agent, None, summary, "");
    }
}

#[derive(Debug, Clone, Default)]
pub struct AgentOutcome {
    pub plan: String,
    pub result: String,
    pub review: Option<SubagentResult>,
    pub test: Option<SubagentResult>,
    pub engineering: String,
}
