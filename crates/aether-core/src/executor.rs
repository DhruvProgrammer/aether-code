//! Executor: stateless high-capability worker that implements via tools (spec §3, §14).
//! Role-aware: a `system_prompt` override and an `allowed_tools` allowlist let the same
//! tool-calling loop serve Coder / Reviewer / Tester / Explorer subagents (spec §7).
//! Also enforces context compaction (§20) and write checkpoints (§15).

use aether_models::{CompletionRequest, Message, ModelProvider, ToolCall};
use aether_permissions::{Permission, Policy};
use aether_sessions::SessionStore;
use aether_tools::{Tool, ToolContext, ToolError, ToolResult};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

pub struct Executor {
    provider: Arc<dyn ModelProvider>,
    model: String,
    tools: HashMap<String, Arc<dyn Tool>>,
    policy: Policy,
    cwd: PathBuf,
    max_iterations: u32,
    context_max_tokens: u32,
    session: Option<Arc<SessionStore>>,
    session_id: String,
    system_prompt: String,
    allowed_tools: Option<Vec<String>>,
}

impl Executor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: Arc<dyn ModelProvider>,
        model: String,
        tools: HashMap<String, Arc<dyn Tool>>,
        policy: Policy,
        cwd: PathBuf,
        max_iterations: u32,
        context_max_tokens: u32,
        session: Option<Arc<SessionStore>>,
        session_id: String,
        system_prompt: String,
        allowed_tools: Option<Vec<String>>,
    ) -> Self {
        Self {
            provider,
            model,
            tools,
            policy,
            cwd,
            max_iterations,
            context_max_tokens,
            session,
            session_id,
            system_prompt,
            allowed_tools,
        }
    }

    pub async fn run(&self, task: &str) -> Result<String> {
        let mut messages = vec![
            Message {
                role: "system".into(),
                content: self.system_prompt.clone(),
                ..Default::default()
            },
            Message {
                role: "user".into(),
                content: task.into(),
                ..Default::default()
            },
        ];

        // Context compaction (spec §20): bound the transcript before each model call.
        messages = compact_messages(messages, self.context_max_tokens);

        let tool_schemas: Vec<Value> = self
            .tools
            .values()
            .filter(|t| {
                self.allowed_tools
                    .as_ref()
                    .map_or(true, |allow| allow.contains(&t.name().to_string()))
            })
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": { "name": t.name(), "description": t.description(), "parameters": t.json_schema() }
                })
            })
            .collect();
        let has_tools = !tool_schemas.is_empty();

        for _step in 0..self.max_iterations {
            let req = CompletionRequest {
                model: self.model.clone(),
                messages: messages.clone(),
                tools: if has_tools { Some(tool_schemas.clone()) } else { None },
                ..Default::default()
            };
            let resp = self.provider.complete(req).await?;

            if resp.tool_calls.is_empty() && resp.content.is_none() {
                return Ok("The model returned an empty response; no actions were taken.".into());
            }

            if resp.tool_calls.is_empty() {
                return Ok(resp.content.unwrap_or_default());
            }

            let tcs: Vec<Value> = resp
                .tool_calls
                .iter()
                .map(|tc| {
                    serde_json::json!({
                        "id": tc.id,
                        "type": "function",
                        "function": { "name": tc.name, "arguments": tc.arguments.to_string() }
                    })
                })
                .collect();
            messages.push(Message {
                role: "assistant".into(),
                content: resp.content.clone().unwrap_or_default(),
                tool_calls: Some(tcs),
                ..Default::default()
            });

            for tc in &resp.tool_calls {
                let out = match self.execute_tool(tc).await {
                    Ok(r) => r.output,
                    Err(e) => format!("ERROR: {e}"),
                };
                messages.push(Message {
                    role: "tool".into(),
                    content: format!("[{}]\n{}", tc.name, out),
                    tool_call_id: Some(tc.id.clone()),
                    ..Default::default()
                });
            }
            messages = compact_messages(messages, self.context_max_tokens);
        }
        Ok("Max iterations reached without a final answer.".into())
    }

    async fn execute_tool(&self, tc: &ToolCall) -> Result<ToolResult, ToolError> {
        if let Some(allow) = &self.allowed_tools {
            if !allow.contains(&tc.name) {
                return Err(ToolError::Other(format!(
                    "tool '{}' not permitted for this role",
                    tc.name
                )));
            }
        }

        // Write checkpoint before mutating files (spec §15).
        if tc.name == "write_file" {
            if let Some(path) = tc.arguments.get("path").and_then(|v| v.as_str()) {
                let full = self.cwd.join(path);
                let before = std::fs::read_to_string(&full).ok();
                if let Some(store) = &self.session {
                    let _ = store.add_checkpoint(&self.session_id, &tc.name, path, before.as_deref());
                }
            }
        }

        let tool = self
            .tools
            .get(&tc.name)
            .ok_or_else(|| ToolError::Other(format!("unknown tool: {}", tc.name)))?;

        let category = tool.category();
        let policy_perm = if category == "bash" {
            let cmd = tc.arguments.get("command").and_then(|v| v.as_str()).unwrap_or("");
            self.policy.check_bash(cmd)
        } else {
            self.policy.value_for(category)
        };
        let effective = match (policy_perm, tool.required_permission()) {
            (Permission::Deny, _) | (_, Permission::Deny) => Permission::Deny,
            (Permission::Ask, _) | (Permission::Allow, Permission::Ask) => Permission::Ask,
            (Permission::Allow, Permission::Allow) => Permission::Allow,
        };

        match effective {
            Permission::Deny => {
                return Err(ToolError::Other(format!("permission denied by policy: {category}")));
            }
            Permission::Ask => match decide_permission(category, &tc.name) {
                Permission::Allow => {}
                _ => {
                    return Err(ToolError::Other(format!(
                        "permission denied by user: {category} `{}`",
                        tc.name
                    )));
                }
            },
            Permission::Allow => {}
        }

        let ctx = ToolContext { cwd: self.cwd.clone() };
        let res = tool.execute(tc.arguments.clone(), &ctx).await;

        if let Some(store) = &self.session {
            let args = tc.arguments.to_string();
            let payload = match &res {
                Ok(r) => r.output.clone(),
                Err(e) => format!("ERROR: {e}"),
            };
            let _ = store.add_tool_call(&self.session_id, &tc.name, &args, &payload);
        }
        res
    }
}

fn estimate_tokens(msgs: &[Message]) -> usize {
    msgs.iter().map(|m| m.content.chars().count() / 4 + 16).sum()
}

/// Resolve a `Permission::Ask`:
/// - **bash** keeps fail-open semantics in non-TTY environments because `Policy::check_bash`
///   already hard-denies catastrophic commands. Safe bash falls through to the configured
///   permission and we don't block non-interactive runs (CI, `--background`, piped input).
/// - **All other categories** (`edit`, `delete`, `git_commit`, `network`) deny by default when
///   stdin is not a TTY. There is no automatic safe answer for "may the agent overwrite a
///   file or push to a network host" without a human; failing open was a silent footgun.
/// - On a TTY, prompt the user with a y/N question.
fn decide_permission(category: &str, tool: &str) -> Permission {
    if !std::io::stdin().is_terminal() {
        return if category == "bash" {
            Permission::Allow
        } else {
            Permission::Deny
        };
    }
    eprint!("[ASK] allow {category} `{tool}`? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    let ans = s.trim().to_ascii_lowercase();
    if ans == "y" || ans == "yes" {
        Permission::Allow
    } else {
        Permission::Deny
    }
}

/// Compaction (spec §20): keep system + first user + recent tail; truncate long tool outputs.
fn compact_messages(mut msgs: Vec<Message>, max_tokens: u32) -> Vec<Message> {
    let max = max_tokens as usize;
    if estimate_tokens(&msgs) <= max {
        return msgs;
    }
    let system = msgs.iter().find(|m| m.role == "system").cloned();
    let first_user = msgs.iter().find(|m| m.role == "user").cloned();
    let mut tail: Vec<Message> = msgs.drain(..).rev().take(10).collect();
    tail.reverse();

    let mut out = Vec::new();
    if let Some(s) = system {
        out.push(s);
    }
    if let Some(u) = first_user {
        out.push(u);
    }
    for m in tail {
        if out.iter().any(|x| x.role == m.role && x.content == m.content) {
            continue;
        }
        out.push(m);
    }

    if estimate_tokens(&out) > max {
        for m in out.iter_mut() {
            if m.role == "tool" && m.content.chars().count() > 2000 {
                let truncated: String = m.content.chars().take(2000).collect();
                m.content = format!("{truncated}…[truncated]");
            }
        }
    }
    out
}
