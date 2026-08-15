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

/// Extract the **last top-level balanced JSON object** from model text. The text may be
/// wrapped in markdown code fences (` ```json ... ``` `), surrounded by prose, or contain
/// extra objects (e.g. a tool-call JSON followed by a handoff JSON). Returns the slice
/// `[start, end]` of the balanced object, or `None` if no balanced object can be found.
///
/// BUG-P1-01, P1-02 regression: the previous implementation used `t.rfind('}')` which could
/// either capture too much (multiple objects) or fail on truncated responses, leading to a
/// silent false-positive (`status = "ok"`) when the agent emitted an unparseable handoff.
pub(crate) fn extract_json_text(text: &str) -> Option<String> {
    let t = text.trim();

    // 1) Try a fenced code block first (```json ... ```).
    if let Some(start) = t.find("```") {
        let after = &t[start + 3..];
        let after = after.trim_start_matches("json").trim_start_matches("JSON").trim_start();
        if let Some(end) = after.find("```") {
            let body = after[..end].trim();
            if let Some(s) = last_balanced_json_object(body) {
                return Some(s);
            }
        }
    }

    // 2) Fall back to scanning the entire text for a balanced top-level object.
    last_balanced_json_object(t)
}

/// Walk `s` from left to right and return the **last** slice `[i, j]` such that `s[i..=j]`
/// is a balanced top-level `{...}` (respecting string literals + escapes). Returns None
/// if no such object exists. Shared by `extract_json_text` and the visual review parser.
pub(crate) fn last_balanced_json_object(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut last: Option<(usize, usize)> = None;
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut escape = false;
    let mut start: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if in_str {
            match b {
                b'\\' => escape = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(a) = start {
                            last = Some((a, i));
                            start = None;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    last.map(|(a, b)| s[a..=b].to_string())
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

    let out = match extract_json_text(&text).and_then(|j| serde_json::from_str::<RawOut>(&j).ok()) {
        Some(r) => SubagentResult {
            role: role.name.to_string(),
            status: if r.status.is_empty() { "ok".into() } else { r.status },
            summary: r.summary,
            findings: r.findings,
            files: r.files,
            raw: text.clone(),
        },
        None => SubagentResult {
            role: role.name.to_string(),
            // BUG-P1-01 regression: a missing/unparseable JSON handoff is NOT a success.
            // Report `unparseable` so the orchestrator fails the verification instead of
            // silently treating a truncated/non-JSON response as "ok".
            status: "unparseable".into(),
            summary: text.chars().take(500).collect(),
            findings: vec![format!(
                "role '{}' produced no balanced JSON handoff (truncated or non-JSON response)",
                role.name
            )],
            files: vec![],
            raw: text.clone(),
        },
    };

    if let Some(store) = &session {
        let _ = store.add_message(session_id, &format!("[{}]", role.name.to_uppercase()), &text);
    }
    Ok(out)
}

/// Convenience: build a `Value` summary of a role result for the orchestrator prompt.
pub fn result_to_value(r: &SubagentResult) -> Value {
    serde_json::json!(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_handles_fences_and_prose() {
        let fenced = "Sure!\n```json\n{\"status\":\"ok\",\"summary\":\"x\"}\n```";
        assert_eq!(
            extract_json_text(fenced),
            Some("{\"status\":\"ok\",\"summary\":\"x\"}".to_string())
        );
        let prose = "Here is the result: {\"status\":\"failed\",\"summary\":\"bad\"} done.";
        assert_eq!(
            extract_json_text(prose),
            Some("{\"status\":\"failed\",\"summary\":\"bad\"}".to_string())
        );
    }

    #[test]
    fn extract_json_picks_last_balanced_object() {
        // Two objects: the handoff JSON is the second one.
        let text = "{\"tool\":\"x\"}\nthen\n{\"status\":\"ok\",\"summary\":\"done\"}";
        assert_eq!(
            extract_json_text(text),
            Some("{\"status\":\"ok\",\"summary\":\"done\"}".to_string())
        );
    }

    #[test]
    fn extract_json_handles_nested_braces() {
        // Nested JSON object — braces inside strings must not confuse the parser.
        let text = "result: {\"status\":\"ok\",\"summary\":\"a{b}c\",\"findings\":[]}";
        assert_eq!(
            extract_json_text(text).unwrap(),
            "{\"status\":\"ok\",\"summary\":\"a{b}c\",\"findings\":[]}"
        );
    }

    #[test]
    fn extract_json_none_on_truncated() {
        // Truncated fenced block — no closing brace.
        let text = "```json\n{\"status\":\"ok\",\"summary\":\"truncated";
        assert_eq!(extract_json_text(text), None);
    }

    #[test]
    fn extract_json_none_on_no_json() {
        assert_eq!(extract_json_text("just plain text, no json"), None);
    }

    #[test]
    fn last_balanced_handles_escaped_quotes_in_string() {
        // String contains an escaped quote + a literal `{` — neither should confuse the scanner.
        let text = r#"{"description":"he said \"{\" and left","score":92}"#;
        assert_eq!(
            last_balanced_json_object(text).as_deref(),
            Some(r#"{"description":"he said \"{\" and left","score":92}"#)
        );
    }

    #[test]
    fn last_balanced_returns_last_when_multiple_objects() {
        // Two top-level objects separated by prose — the last one wins.
        let text = r#"{"a":1} some chatter {"b":2}"#;
        assert_eq!(last_balanced_json_object(text).as_deref(), Some(r#"{"b":2}"#));
    }

    #[test]
    fn last_balanced_deeply_nested() {
        let text = r#"{"a":{"b":{"c":1}}}"#;
        assert_eq!(last_balanced_json_object(text).as_deref(), Some(text));
    }

    #[test]
    fn last_balanced_none_on_empty_or_unbalanced() {
        assert_eq!(last_balanced_json_object(""), None);
        assert_eq!(last_balanced_json_object("{"), None);
        assert_eq!(last_balanced_json_object("not json"), None);
    }
}
