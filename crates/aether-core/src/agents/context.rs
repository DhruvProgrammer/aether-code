//! Agent context builder (spec §29). Assemble only relevant context for one agent — never the
//! entire conversation. Keeps the BIG LLM's context small and high-signal.

use crate::agents::definition::AgentDefinition;
use crate::mode::Mode;

/// Build a focused context block for `def` given the task, current mode, a trimmed engineering
/// state, and any retrieved memory.
pub fn build(def: &AgentDefinition, task: &str, mode: Mode, eng_state: &str, memory: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("# Agent: {} ({})\n", def.name, def.role));
    s.push_str(&format!("Mode: {}\n", mode.label()));
    s.push_str(&format!("Model: {}\n", if def.uses_big_llm() { "BIG/executor" } else { "SMALL/controller" }));
    if !eng_state.trim().is_empty() {
        s.push_str(&format!("\n## Engineering state\n{eng_state}\n"));
    }
    if !memory.trim().is_empty() {
        s.push_str(&format!("\n## Relevant memory\n{memory}\n"));
    }
    s.push_str(&format!("\n## Task\n{task}\n"));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::registry::AgentRegistry;

    #[test]
    fn context_includes_role_and_task() {
        let reg = AgentRegistry::builtin();
        let def = reg.find("explorer").unwrap();
        let c = build(def, "find the auth code", Mode::Build, "conf 0.5", "mem x");
        assert!(c.contains("Explorer"));
        assert!(c.contains("find the auth code"));
        assert!(c.contains("SMALL/controller"));
    }
}
