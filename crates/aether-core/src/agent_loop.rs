//! Top-level agent loop: Controller plans (with memory + repo context), Coder (Executor)
//! implements — with cost-aware model routing (§8) — then optional Reviewer/Tester
//! subagents run a structured handoff pass (spec §4, §7, §29).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use aether_mind::Mind;
use aether_models::ModelProvider;
use aether_permissions::Policy;
use aether_sessions::SessionStore;
use aether_tools::Tool;
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

        // Assemble context: repo instructions + retrieved memory (spec §12, §9.7).
        let mut context = aether_mind::context::discover_context(&self.cwd);
        if let Some(mind) = &self.mind {
            if let Ok(mem) = mind.retrieve(task, self.embedder.as_deref(), self.memory_top_k).await {
                if !mem.is_empty() {
                    context.push_str(&format!("\n## Retrieved Memory\n{}", mem));
                }
            }
        }

        let plan =
            crate::controller::plan(self.controller.as_ref(), &self.controller_model, task, &context).await?;
        if let Some(store) = &self.session {
            let _ = store.add_message(&self.session_id, "assistant", &format!("[PLAN]\n{plan}"));
        }
        println!("[PLAN]\n{plan}\n");

        // Cost routing (§8): pick Coder model by task intent.
        let coder_model = crate::router::select_model(
            task,
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
        let result = coder.run(task).await?;

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

        let final_result = format!("{result}\n{}", self.handoff_text(&review, &test));
        if let Some(store) = &self.session {
            let _ = store.add_message(&self.session_id, "assistant", &final_result);
            let _ = store.record_run(&self.session_id, task, &plan, &final_result);
        }

        // Best-effort memory extraction (spec §9.3). Never blocks the user.
        if self.auto_extract {
            if let (Some(mind), Some(prov)) = (&self.mind, &self.embedder) {
                let transcript = format!("task: {}\nplan: {}\nresult: {}", task, plan, final_result);
                let _ = aether_mind::extract::extract(mind, &transcript, prov.as_ref(), &self.controller_model).await;
            }
        }

        Ok(AgentOutcome { plan, result: final_result, review, test })
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

#[derive(Debug, Clone, Default)]
pub struct AgentOutcome {
    pub plan: String,
    pub result: String,
    pub review: Option<SubagentResult>,
    pub test: Option<SubagentResult>,
}
