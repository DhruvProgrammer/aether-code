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
    // ---- v0.12 optional subsystems ----
    /// Agent id used for per-agent permission scoping.
    pub(crate) agent_id: Option<String>,
    pub(crate) permission_engine: Option<Arc<aether_permissions::PermissionEngine>>,
    pub(crate) context_manager: Option<Arc<aether_context::ContextManager>>,
    /// Session compactor (structured checkpoint compaction). When present,
    /// preflight context estimation + automatic compaction + overflow recovery
    /// replace the legacy `compact_messages` heuristic.
    pub(crate) compactor: Option<Arc<aether_context::SessionCompactor>>,
    /// Optional runtime event sink (spec §4): typed events for tools, files,
    /// context health, and compaction lifecycle. Never required.
    pub(crate) runtime_events: Option<Arc<dyn Fn(crate::task_state::TaskEventKind) + Send + Sync>>,
    /// Task id used for runtime event correlation.
    pub(crate) task_id: Option<String>,
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
            agent_id: None,
            permission_engine: None,
            context_manager: None,
            compactor: None,
            runtime_events: None,
            task_id: None,
        }
    }

    pub fn with_agent_id(mut self, id: impl Into<String>) -> Self { self.agent_id = Some(id.into()); self }
    pub fn with_permission_engine(mut self, e: Arc<aether_permissions::PermissionEngine>) -> Self { self.permission_engine = Some(e); self }
    pub fn with_context_manager(mut self, c: Arc<aether_context::ContextManager>) -> Self { self.context_manager = Some(c); self }
    pub fn with_compactor(mut self, c: Arc<aether_context::SessionCompactor>) -> Self { self.compactor = Some(c); self }

    /// Inject a typed runtime-event sink (spec §4). Events are best-effort;
    /// a missing sink simply means no events.
    pub fn with_runtime_events(
        mut self,
        sink: Arc<dyn Fn(crate::task_state::TaskEventKind) + Send + Sync>,
        task_id: impl Into<String>,
    ) -> Self {
        self.runtime_events = Some(sink);
        self.task_id = Some(task_id.into());
        self
    }

    /// Emit a tool lifecycle event with the stored task id.
    fn emit_tool(&self, ev: impl FnOnce(&str) -> crate::task_state::TaskEventKind) {
        if let (Some(sink), Some(task_id)) = (&self.runtime_events, &self.task_id) {
            sink(ev(task_id));
        }
    }

    pub async fn run(&self, task: &str) -> Result<String> {
        let system_content = crate::prompt::system_for(&self.system_prompt);
        let mut messages = vec![
            Message {
                role: "system".into(),
                content: system_content.clone(),
                ..Default::default()
            },
            Message {
                role: "user".into(),
                content: task.into(),
                ..Default::default()
            },
        ];

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

        // Context compaction (spec §20): bound the transcript before each model call.
        // When a SessionCompactor is configured, preflight estimation + structured
        // checkpoint compaction is used; otherwise the legacy heuristic applies.
        messages = self.preflight_compact(messages, &system_content, &tool_schemas).await;

        for _step in 0..self.max_iterations {
            let req = CompletionRequest {
                model: self.model.clone(),
                messages: messages.clone(),
                tools: if has_tools { Some(tool_schemas.clone()) } else { None },
                ..Default::default()
            };
            let resp = match self.provider.complete(req).await {
                Ok(r) => r,
                Err(e) => {
                    // Context overflow recovery: compact once, retry once.
                    if is_context_overflow(&e) {
                        if let Some(compactor) = &self.compactor {
                            match compactor
                                .compact(&self.session_id, &system_content, &messages, aether_context::CompactTrigger::Overflow)
                                .await
                            {
                                Ok(rebuilt) => {
                                    messages = rebuilt;
                                    let retry = CompletionRequest {
                                        model: self.model.clone(),
                                        messages: messages.clone(),
                                        tools: if has_tools { Some(tool_schemas.clone()) } else { None },
                                        ..Default::default()
                                    };
                                    // Retry exactly once; no infinite loop.
                                    self.provider.complete(retry).await?
                                }
                                Err(ce) => {
                                    return Err(anyhow::anyhow!(
                                        "context overflow and compaction failed: {ce}"
                                    ));
                                }
                            }
                        } else {
                            return Err(e.into());
                        }
                    } else {
                        return Err(e.into());
                    }
                }
            };

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
                self.emit_tool(|tid| crate::task_state::TaskEventKind::ToolStarted {
                    task_id: tid.into(),
                    tool: tc.name.clone(),
                    operation: tc.arguments.to_string().chars().take(120).collect(),
                });
                let out = match self.execute_tool(tc).await {
                    Ok(r) => {
                        // File-lifecycle events from actual tool results (spec §4 GOOD path).
                        let path = tc
                            .arguments
                            .get("path")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        match tc.name.as_str() {
                            "write_file" => {
                                if let Some(p) = &path {
                                    let ev = if std::path::Path::new(p).exists() {
                                        crate::task_state::TaskEventKind::FileModified {
                                            task_id: String::new(),
                                            path: p.clone(),
                                        }
                                    } else {
                                        crate::task_state::TaskEventKind::FileCreated {
                                            task_id: String::new(),
                                            path: p.clone(),
                                        }
                                    };
                                    self.emit_tool(|tid| match ev {
                                        crate::task_state::TaskEventKind::FileModified { path, .. } => {
                                            crate::task_state::TaskEventKind::FileModified { task_id: tid.into(), path }
                                        }
                                        crate::task_state::TaskEventKind::FileCreated { path, .. } => {
                                            crate::task_state::TaskEventKind::FileCreated { task_id: tid.into(), path }
                                        }
                                        other => other,
                                    });
                                }
                            }
                            "read_file" => {}
                            _ => {}
                        }
                        self.emit_tool(|tid| crate::task_state::TaskEventKind::ToolCompleted {
                            task_id: tid.into(),
                            tool: tc.name.clone(),
                        });
                        r.output
                    }
                    Err(e) => {
                        self.emit_tool(|tid| crate::task_state::TaskEventKind::ToolFailed {
                            task_id: tid.into(),
                            tool: tc.name.clone(),
                            error: e.to_string().chars().take(200).collect(),
                        });
                        format!("ERROR: {e}")
                    }
                };
                messages.push(Message {
                    role: "tool".into(),
                    content: format!("[{}]\n{}", tc.name, out),
                    tool_call_id: Some(tc.id.clone()),
                    ..Default::default()
                });
            }
            messages = self.preflight_compact(messages, &system_content, &tool_schemas).await;
        }
        Ok("Max iterations reached without a final answer.".into())
    }

    /// Preflight context estimation + automatic compaction. When a
    /// [`SessionCompactor`] is configured, estimates the request size against
    /// the model's context window and compacts (via structured checkpoint) if
    /// it approaches the safe limit. Otherwise falls back to the legacy
    /// `compact_messages` heuristic. The original request continues after a
    /// successful compaction — the caller never has to resend it.
    async fn preflight_compact(
        &self,
        messages: Vec<Message>,
        system_content: &str,
        tool_schemas: &[Value],
    ) -> Vec<Message> {
        let Some(compactor) = &self.compactor else {
            return compact_messages(messages, self.context_max_tokens);
        };
        let window = compactor.context_window();
        let estimated = aether_context::checkpoint::estimate_request_tokens(
            system_content,
            tool_schemas,
            &messages,
        );
        // Context health warning (spec §12): safe / warning / critical vs the
        // model's real window. Emitted every preflight when not safe.
        let health = aether_context::checkpoint::context_health(estimated, window);
        if health != aether_context::checkpoint::ContextHealth::Safe {
            let health_str = format!("{health:?}").to_lowercase();
            self.emit_tool(|tid| crate::task_state::TaskEventKind::ContextWarning {
                task_id: tid.into(),
                estimated_tokens: estimated,
                context_window: window,
                health: health_str,
            });
        }
        if !aether_context::checkpoint::should_compact(estimated, window) {
            return messages;
        }
        self.emit_tool(|tid| crate::task_state::TaskEventKind::CompactionStarted {
            task_id: tid.into(),
            trigger: "automatic".into(),
        });
        match compactor
            .compact(&self.session_id, system_content, &messages, aether_context::CompactTrigger::Automatic)
            .await
        {
            Ok(rebuilt) => {
                let after = aether_context::checkpoint::estimate_request_tokens(
                    system_content,
                    tool_schemas,
                    &rebuilt,
                );
                self.emit_tool(|tid| crate::task_state::TaskEventKind::CompactionCompleted {
                    task_id: tid.into(),
                    tokens_before: estimated,
                    tokens_after: after,
                });
                rebuilt
            }
            Err(e) => {
                // Transactional failure: old state kept (spec §16).
                self.emit_tool(|tid| crate::task_state::TaskEventKind::CompactionFailed {
                    task_id: tid.into(),
                    error: e.to_string().chars().take(200).collect(),
                });
                messages
            }
        }
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
            // v0.12: route bash through the new hierarchical engine when present;
            // otherwise fall back to the v0.11 hard-coded classify_bash + policy table.
            if let Some(eng) = &self.permission_engine {
                eng.decide_bash(self.agent_id.as_deref(), cmd).verdict
            } else {
                match aether_permissions::classify_bash(cmd) {
                    aether_permissions::BashLevel::Hard => Permission::Deny,
                    aether_permissions::BashLevel::Soft => Permission::Ask,
                    aether_permissions::BashLevel::Safe => self.policy.value_for("bash"),
                }
            }
        } else {
            // Non-bash tools: prefer the hierarchical engine when configured.
            if let Some(eng) = &self.permission_engine {
                let op = match category {
                    "read" => aether_permissions::Operation::Read,
                    "edit" => aether_permissions::Operation::Write,
                    "delete" => aether_permissions::Operation::Delete,
                    "git_commit" => aether_permissions::Operation::Admin,
                    "network" => aether_permissions::Operation::Network,
                    _ => aether_permissions::Operation::Execute,
                };
                eng.decide(op, &aether_permissions::ResourceScope::Tool { value: tc.name.clone() },
                    aether_permissions::engine::DecisionContext {
                        agent_id: self.agent_id.as_deref(),
                        tool: Some(&tc.name),
                        role: None,
                        reason: None,
                    }).verdict
            } else {
                self.policy.value_for(category)
            }
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

/// Classify a provider error as a context-overflow failure. Used to trigger
/// overflow recovery (compact once, retry once).
fn is_context_overflow(e: &aether_models::ProviderError) -> bool {
    let msg = e.to_string().to_lowercase();
    msg.contains("context length")
        || msg.contains("context_length")
        || msg.contains("maximum context")
        || msg.contains("too many tokens")
        || msg.contains("token limit")
        || msg.contains("max_tokens")
        || msg.contains("context window")
        || msg.contains("reduce the length")
        || msg.contains("413")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_classification_detects_context_errors() {
        let e = aether_models::ProviderError::Api(
            "This model's maximum context length is 128000 tokens".into(),
        );
        assert!(is_context_overflow(&e));
        let e2 = aether_models::ProviderError::Api("reduce the length of your input".into());
        assert!(is_context_overflow(&e2));
        let e3 = aether_models::ProviderError::Api("rate limit exceeded".into());
        assert!(!is_context_overflow(&e3));
    }

    #[test]
    fn legacy_compact_keeps_system_and_recent() {
        let mut msgs = vec![Message {
            role: "system".into(),
            content: "SYS".into(),
            ..Default::default()
        }];
        for i in 0..30 {
            msgs.push(Message {
                role: "user".into(),
                content: format!("m{i} {}", "x".repeat(400)),
                ..Default::default()
            });
        }
        let out = compact_messages(msgs, 500);
        assert_eq!(out[0].role, "system");
        assert!(out.len() <= 12);
    }
}
