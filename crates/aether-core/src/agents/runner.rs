//! Agent runner — the execution primitive behind every agent (conceptually OpenClaude's
//! `AgentTool`, spec §10). Builds an `Executor` with the agent's effective tools + mechanically
//! enforced policy, runs it, and parses the structured `SubagentResult`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use aether_models::ModelProvider;
use aether_permissions::Policy;
use aether_sessions::SessionStore;
use aether_tools::Tool;
use anyhow::Result;
use serde::Deserialize;

use crate::agents::definition::AgentDefinition;
use crate::agents::lifecycle::AgentRun;
use crate::executor::Executor;
use crate::subagents::{extract_json_text, SubagentResult};

#[derive(Deserialize)]
struct RawOut {
    #[serde(default)]
    status: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    findings: Vec<String>,
    #[serde(default)]
    files: Vec<String>,
}

pub async fn run_agent(
    def: &AgentDefinition,
    provider: Arc<dyn ModelProvider>,
    model: &str,
    all_tools: &HashMap<String, Arc<dyn Tool>>,
    base_policy: &Policy,
    cwd: &Path,
    session: Option<Arc<SessionStore>>,
    session_id: &str,
    run: &AgentRun,
    task: &str,
) -> Result<SubagentResult> {
    let all_names: Vec<String> = all_tools.keys().cloned().collect();
    let effective = def.effective_tools(&all_names);
    let policy = def.effective_policy(base_policy);

    let exec = Executor::new(
        provider,
        model.to_string(),
        all_tools.clone(),
        policy,
        cwd.to_path_buf(),
        def.budget.max_tool_calls.max(1),
        120_000,
        session.clone(),
        session_id.to_string(),
        def.system_prompt.clone(),
        Some(effective),
    );

    let text = exec.run(task).await?;

    let mut out = match serde_json::from_str::<RawOut>(&extract_json_text(&text)) {
        Ok(r) => SubagentResult {
            role: def.name.clone(),
            status: if r.status.is_empty() { "ok".into() } else { r.status },
            summary: r.summary,
            findings: r.findings,
            files: r.files,
            raw: text.clone(),
        },
        Err(_) => SubagentResult {
            role: def.name.clone(),
            status: "ok".into(),
            summary: text.chars().take(500).collect(),
            findings: vec![],
            files: vec![],
            raw: text.clone(),
        },
    };

    if let Some(store) = &session {
        let _ = store.add_message(session_id, &format!("[{}:{}]", def.id, run.run_id), &text);
    }
    out.raw = serde_json::to_string(&out).unwrap_or(text);
    Ok(out)
}

/// Convenience: route the agent's `model` field to a provider + model string via the resolver.
/// `resolve` returns `(provider, model_string)` for `"controller"`/`"executor"` keys.
pub async fn run_agent_resolved(
    def: &AgentDefinition,
    resolve: impl Fn(&str) -> (Arc<dyn ModelProvider>, String),
    all_tools: &HashMap<String, Arc<dyn Tool>>,
    base_policy: &Policy,
    cwd: &Path,
    session: Option<Arc<SessionStore>>,
    session_id: &str,
    run: &AgentRun,
    task: &str,
) -> Result<SubagentResult> {
    let (provider, model) = resolve(&def.model);
    run_agent(def, provider, &model, all_tools, base_policy, cwd, session, session_id, run, task).await
}
