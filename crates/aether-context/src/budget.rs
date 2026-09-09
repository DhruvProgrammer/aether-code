//! Token budget for an AETHER model request (OpenCode-inspired).
//!
//! The budget aggregates every component that contributes to a model call
//! and reports its state as `Safe / Warning / Critical` based on the
//! configured model's actual context window. Components are tracked
//! independently so the agent loop can decide which to truncate when the
//! budget is over the warning threshold.
//!
//! The estimator uses the same `chars/4` heuristic as the rest of the crate
//! for consistency; tool-output truncation lives next to the tool runtime.

use aether_models::Message;

/// Each component of a model request that consumes the token budget.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BudgetComponents {
    pub system_prompt: u32,
    pub role_prompt: u32,
    pub skills_index: u32,
    pub tool_schemas: u32,
    pub workspace_context: u32,
    pub memory: u32,
    pub conversation: u32,
    pub tool_results: u32,
    pub checkpoint: u32,
    pub recent_messages: u32,
    pub current_request: u32,
    pub requested_output: u32,
}

impl BudgetComponents {
    pub fn total(&self) -> u32 {
        self.system_prompt
            .saturating_add(self.role_prompt)
            .saturating_add(self.skills_index)
            .saturating_add(self.tool_schemas)
            .saturating_add(self.workspace_context)
            .saturating_add(self.memory)
            .saturating_add(self.conversation)
            .saturating_add(self.tool_results)
            .saturating_add(self.checkpoint)
            .saturating_add(self.recent_messages)
            .saturating_add(self.current_request)
            .saturating_add(self.requested_output)
    }
}

/// Health state of the budget relative to the model window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextHealth {
    Safe,
    Warning,
    Critical,
}

impl ContextHealth {
    pub fn is_safe(&self) -> bool { matches!(self, ContextHealth::Safe) }
}

pub const CONTEXT_WARNING_RATIO: f32 = 0.70;
pub const CONTEXT_CRITICAL_RATIO: f32 = 0.85;

/// Heuristic token estimator: characters / 4 + 16 overhead per message.
pub fn estimate_message_tokens(msgs: &[Message]) -> u32 {
    msgs.iter()
        .map(|m| (m.content.chars().count() as u32) / 4 + 16)
        .sum()
}

pub fn estimate_string_tokens(s: &str) -> u32 {
    (s.chars().count() as u32) / 4 + 16
}

pub fn estimate_json_tokens(v: &serde_json::Value) -> u32 {
    estimate_string_tokens(&v.to_string())
}

/// Build a budget from a model call's inputs.
pub fn build(
    system_prompt: &str,
    role_prompt: &str,
    skills_index: &str,
    tool_schemas: &[serde_json::Value],
    workspace_context: &str,
    memory: &str,
    messages: &[Message],
    checkpoint_summary: &str,
    recent: &[Message],
    current_request: &str,
    requested_output_tokens: u32,
) -> BudgetComponents {
    let tools: u32 = tool_schemas.iter().map(estimate_json_tokens).sum();
    // Best-effort de-dup of `recent` from `messages` by content fingerprint.
    let recent_set: std::collections::HashSet<&str> = recent.iter().map(|m| m.content.as_str()).collect();
    let conv_msgs: Vec<Message> = messages.iter().filter(|m| !recent_set.contains(m.content.as_str())).cloned().collect();
    BudgetComponents {
        system_prompt: estimate_string_tokens(system_prompt),
        role_prompt: estimate_string_tokens(role_prompt),
        skills_index: estimate_string_tokens(skills_index),
        tool_schemas: tools,
        workspace_context: estimate_string_tokens(workspace_context),
        memory: estimate_string_tokens(memory),
        conversation: estimate_message_tokens(&conv_msgs),
        tool_results: 0, // tracked separately by the runtime when tools execute
        checkpoint: estimate_string_tokens(checkpoint_summary),
        recent_messages: estimate_message_tokens(recent),
        current_request: estimate_string_tokens(current_request),
        requested_output: requested_output_tokens,
    }
}

/// Classify the budget against the configured model's actual window.
pub fn classify(components: &BudgetComponents, context_window: u32) -> ContextHealth {
    if context_window == 0 {
        return ContextHealth::Safe;
    }
    let ratio = components.total() as f32 / context_window as f32;
    if ratio >= CONTEXT_CRITICAL_RATIO {
        ContextHealth::Critical
    } else if ratio >= CONTEXT_WARNING_RATIO {
        ContextHealth::Warning
    } else {
        ContextHealth::Safe
    }
}

/// Default cap (chars) for tool output. Larger outputs must be truncated
/// before the model sees them. 50 KB matches OpenCode's typical cap.
pub const TOOL_OUTPUT_DEFAULT_MAX_BYTES: usize = 50 * 1024;
/// Hard cap for tool output truncation (OpenCode: 2 KB before summarization).
pub const TOOL_OUTPUT_SUMMARIZE_AT_BYTES: usize = 2 * 1024;

/// Truncate tool output to a max byte length with a clear notice so the
/// model knows the content was cut. Operates on `&str` and returns `String`.
pub fn truncate_tool_output(output: &str, max_bytes: usize) -> (String, bool) {
    if output.len() <= max_bytes {
        return (output.to_string(), false);
    }
    let mut s = String::with_capacity(max_bytes + 80);
    s.push_str(&output[..max_bytes]);
    s.push_str(&format!(
        "\n\n[truncated: output was {} bytes; only first {} shown to model. Re-run with a narrower query or page through manually.]",
        output.len(),
        max_bytes,
    ));
    (s, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(s: &str) -> Message {
        Message { role: "user".into(), content: s.into(), ..Default::default() }
    }

    #[test]
    fn budget_total_sums_components() {
        let b = BudgetComponents {
            system_prompt: 100, role_prompt: 50, skills_index: 0,
            tool_schemas: 30, workspace_context: 0, memory: 20,
            conversation: 0, tool_results: 10, checkpoint: 5,
            recent_messages: 0, current_request: 40, requested_output: 0,
        };
        assert_eq!(b.total(), 255);
    }

    #[test]
    fn health_thresholds_match_design() {
        let mut c = BudgetComponents::default();
        c.system_prompt = 7000; // 70% of 10k
        assert_eq!(classify(&c, 10_000), ContextHealth::Warning);
        c.system_prompt = 8500;
        assert_eq!(classify(&c, 10_000), ContextHealth::Critical);
        c.system_prompt = 5000;
        assert_eq!(classify(&c, 10_000), ContextHealth::Safe);
    }

    #[test]
    fn build_sums_recent_and_conversation_separately() {
        let msgs = vec![m("aaaaaa"), m("bbbb"), m("cccc")];
        let recent = vec![msgs[2].clone()];
        let b = build("", "", "", &[], "", "", &msgs, "", &recent, "", 0);
        // conversation contains all 3; recent is a separate counter
        assert!(b.recent_messages > 0);
        assert!(b.conversation >= b.recent_messages);
    }

    #[test]
    fn truncate_no_cut_under_limit() {
        let (s, cut) = truncate_tool_output("hello", 100);
        assert_eq!(s, "hello");
        assert!(!cut);
    }

    #[test]
    fn truncate_cuts_over_limit() {
        let big = "x".repeat(200);
        let (s, cut) = truncate_tool_output(&big, 50);
        assert!(cut);
        assert!(s.starts_with(&"x".repeat(50)));
        assert!(s.contains("truncated"));
    }
}
