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
    max_iterations: u32,
    context_max_tokens: u32,
    loop_budget: u32,
    /// LLM 3 — VISUAL FRONTEND REVIEWER (optional). When `Some`, the 3-LLM visual loop may run.
    reviewer: Option<Arc<dyn ModelProvider>>,
    reviewer_model: Option<String>,
    /// Frontend visual-engineering configuration (spec: 3-LLM visual review).
    frontend: FrontendConfig,
    // ---- v0.12 subsystems (all optional; default None) ----
    /// Hierarchical permission engine (v0.12).
    permission_engine: Option<Arc<aether_permissions::PermissionEngine>>,
    /// Per-agent context manager (v0.12). When absent the legacy
    /// `compact_messages` heuristic inside the Executor is used.
    context_manager: Option<Arc<aether_context::ContextManager>>,
    /// Snapshot manager (v0.12).
    snapshots: Option<Arc<std::sync::Mutex<aether_sessions::SnapshotManager>>>,
    // ---- v0.13 subsystems (all optional; default None) ----
    /// Plugin registry + hook bus (v0.13).
    plugins: Option<Arc<aether_plugin::Registry>>,
    /// Evidence bag collected from specialist agents (v0.13). The controller
    /// aggregates evidence before accepting verification.
    evidence: Option<Arc<aether_evidence::EvidenceBag>>,
    /// Agent-aware context workspace (v0.13): per-agent contexts + shared
    /// global segments.
    context_workspace: Option<Arc<aether_context::ContextWorkspace>>,
    /// External cancel signal. When notified, the agent loop returns at the
    /// next iteration boundary. Used by the desktop to abort in-process runs.
    cancel: Option<Arc<tokio::sync::Notify>>,
    /// Session compactor (structured checkpoint compaction). When present, the
    /// Executor uses preflight context estimation + automatic compaction +
    /// overflow recovery instead of the legacy truncation heuristic.
    compactor: Option<Arc<aether_context::SessionCompactor>>,
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
            max_iterations,
            context_max_tokens,
            loop_budget,
            reviewer,
            reviewer_model,
            frontend,
            permission_engine: None,
            context_manager: None,
            snapshots: None,
            plugins: None,
            evidence: None,
            context_workspace: None,
            cancel: None,
            compactor: None,
        }
    }

    /// Builder-style injection for the v0.12 subsystems. All optional.
    pub fn with_permission_engine(mut self, e: Arc<aether_permissions::PermissionEngine>) -> Self {
        self.permission_engine = Some(e);
        self
    }
    pub fn with_context_manager(mut self, c: Arc<aether_context::ContextManager>) -> Self {
        self.context_manager = Some(c);
        self
    }
    pub fn with_snapshots(mut self, s: Arc<std::sync::Mutex<aether_sessions::SnapshotManager>>) -> Self {
        self.snapshots = Some(s);
        self
    }

    /// Builder-style injection for the v0.13 subsystems. All optional.
    pub fn with_plugins(mut self, p: Arc<aether_plugin::Registry>) -> Self {
        self.plugins = Some(p);
        self
    }
    pub fn with_evidence(mut self, e: Arc<aether_evidence::EvidenceBag>) -> Self {
        self.evidence = Some(e);
        self
    }
    pub fn with_context_workspace(mut self, w: Arc<aether_context::ContextWorkspace>) -> Self {
        self.context_workspace = Some(w);
        self
    }

    /// Inject an external cancellation handle. The agent loop returns at the
    /// next iteration boundary once the handle is notified.
    pub fn with_cancel(mut self, handle: Arc<tokio::sync::Notify>) -> Self {
        self.cancel = Some(handle);
        self
    }

    /// Inject the session compactor (structured checkpoint compaction).
    pub fn with_compactor(mut self, c: Arc<aether_context::SessionCompactor>) -> Self {
        self.compactor = Some(c);
        self
    }

    /// Accessor used by the Executor when integrating with the permission
    /// engine.
    pub fn permission_engine(&self) -> Option<&Arc<aether_permissions::PermissionEngine>> {
        self.permission_engine.as_ref()
    }
    pub fn context_manager(&self) -> Option<&Arc<aether_context::ContextManager>> {
        self.context_manager.as_ref()
    }
    pub fn snapshots(&self) -> Option<&Arc<std::sync::Mutex<aether_sessions::SnapshotManager>>> {
        self.snapshots.as_ref()
    }

    /// v0.13 accessors.
    pub fn plugins(&self) -> Option<&Arc<aether_plugin::Registry>> {
        self.plugins.as_ref()
    }
    pub fn evidence(&self) -> Option<&Arc<aether_evidence::EvidenceBag>> {
        self.evidence.as_ref()
    }
    pub fn context_workspace(&self) -> Option<&Arc<aether_context::ContextWorkspace>> {
        self.context_workspace.as_ref()
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
        let sid: &str = resume_session.unwrap_or(&self.session_id);
        if let Some(store) = &self.session {
            if let Err(e) = store.add_message(sid, "user", task) {
                eprintln!("aether: session persist failed (user message): {e}");
            }
        }

        // v0.13: plugin session-start hook.
        if let Some(plugins) = &self.plugins {
            let mut out = aether_plugin::SessionStartOutput::default();
            let _ = plugins
                .on_session_start(
                    &aether_plugin::SessionStartInput {
                        session_id: sid.to_string(),
                        cwd: self.cwd.clone(),
                        resumed: resume_session.is_some(),
                        model: self.controller_model.clone(),
                    },
                    &mut out,
                )
                .await;
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
            if let Some(c) = &self.cancel {
                if tokio::time::timeout(std::time::Duration::ZERO, c.notified()).await.is_ok() {
                    final_result = "[cancelled by caller]".into();
                    break;
                }
            }
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

            // Model 1 always runs on the explicitly configured executor provider.
            // No cost routing, no dynamic selection (v0.15 gateway spec §1, §23).
            let coder_model = self.executor_model.clone();
            let mut coder = Executor::new(
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
            )
            .with_agent_id("coder");
            if let Some(pe) = &self.permission_engine { coder = coder.with_permission_engine(pe.clone()); }
            if let Some(cm) = &self.context_manager { coder = coder.with_context_manager(cm.clone()); }
            if let Some(cp) = &self.compactor { coder = coder.with_compactor(cp.clone()); }
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

                    // v0.13: plugin agent-spawn hook (observability + audit).
                    if let Some(plugins) = &self.plugins {
                        let mut out = aether_plugin::AgentSpawnHookOutput::default();
                        let _ = plugins
                            .on_agent_spawn(
                                &aether_plugin::AgentSpawnHookInput {
                                    agent_id: def.id.clone(),
                                    role: def.role.clone(),
                                    parent: Some("controller".into()),
                                    depth: 1,
                                    task: ctx.clone(),
                                    model: model.clone(),
                                },
                                &mut out,
                            )
                            .await;
                    }

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

                            // v0.13: evidence engine — structured verdicts instead of prose.
                            if let Some(bag) = &self.evidence {
                                let kind = match aid.as_str() {
                                    "tester" => aether_evidence::EvidenceKind::Test,
                                    "security-reviewer" => aether_evidence::EvidenceKind::Security,
                                    _ => aether_evidence::EvidenceKind::Review,
                                };
                                let rec = if r.status == "ok" {
                                    aether_evidence::Recommendation::Pass
                                } else {
                                    aether_evidence::Recommendation::Replan
                                };
                                let mut ev = aether_evidence::Evidence::new(
                                    aid.clone(),
                                    def.role.clone(),
                                    kind,
                                    r.summary.clone(),
                                )
                                .with_recommendation(rec)
                                .with_confidence(aether_evidence::Confidence(if r.status == "ok" { 0.85 } else { 0.4 }));
                                for f in &r.files {
                                    ev = ev.with_file(std::path::PathBuf::from(f));
                                }
                                bag.add(ev);
                            }

                            // v0.13: plugin agent-complete hook.
                            if let Some(plugins) = &self.plugins {
                                let mut out = aether_plugin::AgentCompleteOutput::default();
                                let _ = plugins
                                    .on_agent_complete(
                                        &aether_plugin::AgentCompleteInput {
                                            agent_id: def.id.clone(),
                                            role: def.role.clone(),
                                            status: r.status.clone(),
                                            summary: r.summary.clone(),
                                            findings: r.findings.clone(),
                                            files: r.files.clone(),
                                            latency_ms: 0,
                                            tokens_in: 0,
                                            tokens_out: 0,
                                        },
                                        &mut out,
                                    )
                                    .await;
                            }

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

            // v0.13: evidence-driven decision. Aggregate structured evidence
            // from all specialist agents; surface the verdict + reasoning to
            // the engineering model and (optionally) to plugins.
            if let Some(bag) = &self.evidence {
                let decision = aether_evidence::decide(bag);
                eng.add_decision(
                    &format!("evidence verdict: {:?}", decision.verdict),
                    &decision.reasoning.summary,
                    decision.aggregate_confidence,
                );
                println!("[EVIDENCE] {}", decision.reasoning.summary);
                if let Some(plugins) = &self.plugins {
                    let _ = plugins
                        .publish(
                            aether_plugin::EventKind::Custom("evidence_decision".into()),
                            serde_json::json!({
                                "verdict": format!("{:?}", decision.verdict),
                                "confidence": decision.aggregate_confidence,
                                "agents": decision.contributing_agents,
                            }),
                        )
                        .await;
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

        // v0.13: plugin session-end hook.
        if let Some(plugins) = &self.plugins {
            let mut out = aether_plugin::SessionEndOutput::default();
            let _ = plugins
                .on_session_end(
                    &aether_plugin::SessionEndInput {
                        session_id: sid.to_string(),
                        exit_reason: if escalation.is_some() { "escalated".into() } else { "completed".into() },
                        duration_secs: 0,
                        success: escalation.is_none(),
                    },
                    &mut out,
                )
                .await;
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
        let mut coder = Executor::new(
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
        )
        .with_agent_id("correction-coder");
        if let Some(pe) = &self.permission_engine { coder = coder.with_permission_engine(pe.clone()); }
        if let Some(cm) = &self.context_manager { coder = coder.with_context_manager(cm.clone()); }
        if let Some(cp) = &self.compactor { coder = coder.with_compactor(cp.clone()); }
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
