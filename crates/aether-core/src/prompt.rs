//! AETHER Core System Prompt (behavioral foundation for every role).
//!
//! The authoritative text lives in `crates/aether-core/prompt/aether-core-system-prompt.md`
//! and is compiled into every binary via `include_str!`. It is prepended to the
//! system prompt of every LLM call the runtime owns — Controller, Executor
//! (Coder + subagents), and the Visual Frontend Reviewer — so all roles share
//! the same instruction hierarchy, evidence standards, security rules, and
//! provider policy (no automatic model/provider switching).

/// The full AETHER Core System Prompt, exactly as authored in the markdown file.
pub const AETHER_CORE_SYSTEM_PROMPT: &str =
    include_str!("../prompt/aether-core-system-prompt.md");

/// Build a role system message: core prompt first (highest authority inline),
/// then the role-specific instructions. The core prompt always precedes so
/// role text cannot quietly override it.
pub fn system_for(role_instructions: &str) -> String {
    format!("{AETHER_CORE_SYSTEM_PROMPT}\n\n---\n\n{role_instructions}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_is_embedded_in_full() {
        assert!(AETHER_CORE_SYSTEM_PROMPT.contains("# AETHER Core System Prompt"));
        assert!(AETHER_CORE_SYSTEM_PROMPT.contains("Model 1 — Big Executor"));
        assert!(AETHER_CORE_SYSTEM_PROMPT.contains("Model 2 — Small Controller"));
        assert!(AETHER_CORE_SYSTEM_PROMPT.contains("Model 3 — Visual Frontend Reviewer"));
        assert!(AETHER_CORE_SYSTEM_PROMPT.contains("Do Not Pretend Actions Happened"));
        assert!(AETHER_CORE_SYSTEM_PROMPT.contains("Do not perform cost-based routing."));
        // Sanity: full doc, not truncated.
        assert!(AETHER_CORE_SYSTEM_PROMPT.contains("# End of AETHER Core System Prompt"));
    }

    #[test]
    fn role_instructions_follow_the_core_prompt() {
        let s = system_for("You are the Coder.");
        assert!(s.starts_with("# AETHER Core System Prompt"));
        assert!(s.ends_with("You are the Coder."));
        assert!(s.contains("---"));
    }
}
