//! Subagent framework (spec §7). Specialized roles run the shared Executor with a
//! role-specific system prompt, tool allowlist, and read-only policy. They return a
//! structured `SubagentResult` (JSON handoff) consumed by the orchestrator.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use aether_models::ModelProvider;
use aether_permissions::{Permission, Policy};
use aether_sessions::SessionStore;
use aether_tools::Tool;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::executor::Executor;

/// Structured handoff artifact returned by every role (spec §7).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentResult {
    pub role: String,
    pub status: String, // "ok" | "changes_requested" | "failed"
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    /// Raw model text (kept for debugging / non-JSON fallbacks).
    #[serde(default)]
    pub raw: String,
}

#[derive(Debug, Clone)]
pub struct Role {
    pub name: &'static str,
    pub system: &'static str,
    /// `None` = all tools available; otherwise an explicit allowlist of tool names.
    pub allows: Option<&'static [&'static str]>,
}

pub const EXPLORER: Role = Role {
    name: "explorer",
    system: "You are the Explorer. Map the repository read-only: locate relevant files, \
             understand structure, and report findings. Never modify anything. \
             Return JSON: {\"status\":\"ok\",\"summary\":string,\"findings\":[string],\"files\":[string]}.",
    allows: Some(&[
        "read_file", "list_directory", "grep", "git_status", "git_diff", "git_log", "git_branch",
    ]),
};

pub const REVIEWER: Role = Role {
    name: "reviewer",
    system: "You are the Reviewer. Critically review the changes for correctness, security, and \
             style. Never modify files. Return JSON: \
             {\"status\":\"ok\"|\"changes_requested\",\"summary\":string,\"findings\":[string],\"files\":[string]}.",
    allows: Some(&["read_file", "list_directory", "grep", "git_diff", "git_log"]),
};

pub const TESTER: Role = Role {
    name: "tester",
    system: "You are the Tester. Run the project's test suite and report results. You may execute \
             read-only/test commands only. Return JSON: \
             {\"status\":\"ok\"|\"failed\",\"summary\":string,\"findings\":[string],\"files\":[string]}.",
    allows: Some(&["read_file", "list_directory", "grep", "execute_command", "git_status", "git_diff"]),
};

/// Derive a policy for a role from the base policy (spec §14): read-only roles cannot
/// edit/delete/commit; the tester may run commands but not edit.
pub fn role_policy(role: &Role, base: &Policy) -> Policy {
    match role.name {
        "explorer" | "reviewer" => Policy {
            read: base.read,
            edit: Permission::Deny,
            delete: Permission::Deny,
            bash: Permission::Ask,
            git_commit: Permission::Deny,
            network: Permission::Ask,
        },
        "tester" => Policy {
            read: base.read,
            edit: Permission::Deny,
            delete: Permission::Deny,
            bash: Permission::Allow,
            git_commit: Permission::Ask,
            network: Permission::Ask,
        },
        _ => base.clone(),
    }
}

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

/// Run a single role to completion and parse its structured handoff.
pub async fn run_role(
    role: &Role,
    provider: Arc<dyn ModelProvider>,
    model: &str,
    tools: &HashMap<String, Arc<dyn Tool>>,
    base_policy: &Policy,
    cwd: &Path,
    session: Option<Arc<SessionStore>>,
    session_id: &str,
    task: &str,
) -> anyhow::Result<SubagentResult> {
    let policy = role_policy(role, base_policy);
    let allowed = role.allows.map(|a| a.iter().map(|s| s.to_string()).collect::<Vec<_>>());

    let exec = Executor::new(
        provider,
        model.to_string(),
        tools.clone(),
        policy,
        cwd.to_path_buf(),
        20,
        120_000,
        session.clone(),
        session_id.to_string(),
        role.system.to_string(),
        allowed,
    );

    let text = exec.run(task).await?;

    let mut out = match serde_json::from_str::<RawOut>(&text) {
        Ok(r) => SubagentResult {
            role: role.name.to_string(),
            status: if r.status.is_empty() { "ok".into() } else { r.status },
            summary: r.summary,
            findings: r.findings,
            files: r.files,
            raw: text.clone(),
        },
        Err(_) => SubagentResult {
            role: role.name.to_string(),
            status: "ok".into(),
            summary: text.chars().take(500).collect(),
            findings: vec![],
            files: vec![],
            raw: text.clone(),
        },
    };

    if let Some(store) = &session {
        let _ = store.add_message(session_id, &format!("[{}]", role.name.to_uppercase()), &text);
    }
    // Surface handoff as JSON for downstream logging/aggregation.
    out.raw = serde_json::to_string(&out).unwrap_or(text);
    Ok(out)
}

/// Convenience: build a `Value` summary of a role result for the orchestrator prompt.
pub fn result_to_value(r: &SubagentResult) -> Value {
    serde_json::json!(r)
}
