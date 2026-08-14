//! Top-level agent loop: Controller plans (with memory + repo context), Coder (Executor)
//! implements — with cost-aware model routing (§8) — then optional Reviewer/Tester
//! subagents run a structured handoff pass (spec §4, §7, §29).
//!
//! Wrapped by a *loop-engineering* outer loop: an explicit `EngineeringModel` is
//! maintained across plan→execute→verify→replan cycles. The loop engine detects
//! stagnation, enforces budgets, tracks hypotheses/evidence/confidence, and acts as
//! a circuit breaker that escalates or stops instead of retrying blindly.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use aether_mind::Mind;
use aether_models::ModelProvider;
use aether_permissions::Policy;
use aether_sessions::SessionStore;
use aether_tools::Tool;
use crate::eng::{LoopAction, LoopEngine, LoopState};
use crate::executor::Executor;
use crate::subagents::{run_role, SubagentResult, REVIEWER, TESTER};

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
    reviewer_model: String,
    tester_model: String,
    cheap_model: Option<String>,
    max_iterations: u32,
    context_max_tokens: u32,
    loop_budget: u32,
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
        reviewer_model: String,
        tester_model: String,
        cheap_model: Option<String>,
        max_iterations: u32,
        context_max_tokens: u32,
        loop_budget: u32,
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
            reviewer_model,
            tester_model,
            cheap_model,
            max_iterations,
            context_max_tokens,
            loop_budget,
        }
    }

    fn provider_for(&self, key: &str) -> Arc<dyn ModelProvider> {
        self.providers
            .get(key)
            .cloned()
            .or_else(|| self.providers.get(&self.executor_model).cloned())
            .unwrap_or_else(|| self.controller.clone())
    }

    pub async fn run(&self, task: &str) -> anyhow::Result<AgentOutcome> {
        if let Some(store) = &self.session {
            let _ = store.add_message(&self.session_id, "user", task);
        }

        // --- Loop engineering: establish the EngineeringModel -------------------
        let mut eng = LoopEngine::new(task);
        eng.set_success_criteria(vec!["reviewer passes".into(), "tester passes".into()]);
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
                None => task.to_string(),
                Some(prev) => format!(
                    "{task}\n\n# Prior attempt result (adapt — do not repeat past failures)\n{prev}\n\n# Engineering state\n{}\n",
                    eng.state_summary()
                ),
            };

            // Plan (or replan) with the model-informed task.
            let plan = crate::controller::plan(
                self.controller.as_ref(),
                &self.controller_model,
                &cycle_task,
                &context,
            )
            .await?;
            if let Some(store) = &self.session {
                let _ = store.add_message(&self.session_id, "assistant", &format!("[PLAN {}]\n{plan}", iter + 1));
            }
            println!("[PLAN {}]\n{plan}\n", iter + 1);
            eng.set_strategy(&plan);
            eng.add_decision(&format!("plan iteration {}", iter + 1), "controller produced plan", 0.6);

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
                self.session_id.clone(),
                CODER_SYSTEM.to_string(),
                None,
            );
            let result = coder.run(&cycle_task).await?;
            eng.record_action("execute plan via tools");
            eng.observe("executor", &summarize(&result), None, None);

            // Subagent handoff: Reviewer + Tester (spec §7).
            let mut review: Option<SubagentResult> = None;
            let mut test: Option<SubagentResult> = None;
            if self.subagents_enabled {
                let ctx = format!("Original task:\n{task}\n\nImplementation result:\n{result}");
                if !self.reviewer_model.is_empty() {
                    let p = self.provider_for(&self.reviewer_model);
                    match run_role(
                        &REVIEWER,
                        p,
                        &self.reviewer_model,
                        &self.tools,
                        &self.policy,
                        &self.cwd,
                        self.session.clone(),
                        &self.session_id,
                        &ctx,
                    )
                    .await
                    {
                        Ok(r) => {
                            println!("[REVIEW] {}\n", r.summary);
                            review = Some(r);
                        }
                        Err(e) => eprintln!("reviewer failed: {e}"),
                    }
                }
                if !self.tester_model.is_empty() {
                    let p = self.provider_for(&self.tester_model);
                    match run_role(
                        &TESTER,
                        p,
                        &self.tester_model,
                        &self.tools,
                        &self.policy,
                        &self.cwd,
                        self.session.clone(),
                        &self.session_id,
                        &ctx,
                    )
                    .await
                    {
                        Ok(r) => {
                            println!("[TEST] {}\n", r.summary);
                            test = Some(r);
                        }
                        Err(e) => eprintln!("tester failed: {e}"),
                    }
                }
            }
            last_review = review.clone();
            last_test = test.clone();

            // Update the EngineeringModel from verification evidence.
            if self.subagents_enabled {
                if let Some(r) = &review {
                    let pass = r.status == "ok";
                    if pass {
                        eng.mark_criteria_met("reviewer passes");
                        eng.add_evidence(&format!("review: {}", r.summary), "review", 0.9, None, None);
                    } else {
                        eng.note_failure(&format!("reviewer: {}", r.summary));
                        eng.add_evidence(&format!("review: {}", r.summary), "review", 0.3, None, None);
                    }
                    for f in &r.findings {
                        eng.add_unknown(f);
                    }
                }
                if let Some(t) = &test {
                    let pass = t.status == "ok";
                    if pass {
                        eng.mark_criteria_met("tester passes");
                        eng.add_evidence(&format!("test: {}", t.summary), "test", 0.9, None, None);
                    } else {
                        eng.note_failure(&format!("tester: {}", t.summary));
                        eng.add_evidence(&format!("test: {}", t.summary), "test", 0.3, None, None);
                    }
                }
            } else {
                // No verifier configured: avoid redundant re-execution; close after one cycle.
                eng.mark_criteria_met("reviewer passes");
                eng.mark_criteria_met("tester passes");
            }

            // Next-best-action guidance for the next cycle (or the human).
            if eng.detect_stagnation() {
                eng.set_next_best_action("STOP — approach is not converging; escalate to human");
            } else if !eng.model.unknowns.is_empty() {
                eng.set_next_best_action(&format!("Resolve open unknown: {}", eng.model.unknowns.last().unwrap()));
            } else {
                eng.set_next_best_action("Continue implementing remaining plan steps");
            }

            // Persist the engineering model for inspection / resume.
            if let Some(store) = &self.session {
                let _ = store.set_kv(
                    &self.session_id,
                    "engineering",
                    &serde_json::to_string(&eng.model).unwrap_or_default(),
                );
            }

            final_result = format!("{result}\n{}", self.handoff_text(&review, &test));
            println!("{}", eng.render_panel());

            match eng.decide(iter + 1, loop_budget) {
                LoopAction::Escalate => {
                    escalation = Some(eng.escalation_briefing());
                    break;
                }
                LoopAction::Stop => break,
                LoopAction::Continue => {
                    prev_result = Some(final_result.clone());
                    continue;
                }
            }
        }

        if let Some(store) = &self.session {
            let _ = store.add_message(&self.session_id, "assistant", &final_result);
            let _ = store.record_run(
                &self.session_id,
                task,
                &eng.model.current_strategy.clone().unwrap_or_default(),
                &final_result,
            );
        }

        // Best-effort memory extraction (spec §9.3). Never blocks the user.
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

    fn handoff_text(&self, review: &Option<SubagentResult>, test: &Option<SubagentResult>) -> String {
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
        s
    }
}

fn summarize(s: &str) -> String {
    s.lines().take(12).collect::<Vec<_>>().join("\n")
}

#[derive(Debug, Clone, Default)]
pub struct AgentOutcome {
    pub plan: String,
    pub result: String,
    pub review: Option<SubagentResult>,
    pub test: Option<SubagentResult>,
    pub engineering: String,
}
