//! Execution modes (BUILD / PLAN) and the default Karpathy engineering guidelines.
//!
//! This is a thin behavioral layer that sits on top of the existing agent (the two-LLM
//! system, tools, memory, subagents, and the loop-engineering `eng` module). It does not
//! redesign the agent — it only changes what the Controller/Executor are told to do and
//! which operations are permitted (spec: build_plan_modes).

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Default. Understand, plan, modify, execute, verify.
    #[default]
    Build,
    /// Read-only. Investigate the repository and produce an implementation plan; never modify.
    Plan,
}

impl Mode {
    pub fn label(&self) -> &'static str {
        match self {
            Mode::Build => "BUILD",
            Mode::Plan => "PLAN",
        }
    }

    pub fn is_plan(&self) -> bool {
        matches!(self, Mode::Plan)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Karpathy engineering guidelines — the agent's default behavioral policy, applied to
/// BOTH modes (spec §28). Injected into Controller/Executor prompts; the full
/// authoritative text lives in `skills/karpathy-guidelines/SKILL.md`.
pub const KARPATHY_POLICY: &str = "\
Follow the Karpathy Guidelines: \
(1) Think before coding — surface ambiguity, state assumptions, prefer the simpler approach. \
(2) Simplicity first — minimum code that solves the actual problem; avoid speculative abstractions and unnecessary dependencies. \
(3) Surgical changes — touch only what is necessary and match existing style. \
(4) Goal-driven execution — translate requests into verifiable success criteria and confirm them.";

/// System prompt fragment for BUILD MODE (spec §38).
pub const BUILD_MODE_PROMPT: &str = "You are operating in BUILD MODE. Implement the user's \
requested change and verify it. Do not modify unrelated code, do not invent unnecessary \
architecture, and do not claim completion without verification.";

/// System prompt fragment for PLAN MODE (spec §39).
pub const PLAN_MODE_PROMPT: &str = "You are operating in PLAN MODE. Understand the requested \
change and produce an implementation-ready plan. Do NOT modify application source code. \
Investigate the repository, separate facts/assumptions/unknowns/hypotheses, and include \
verification steps.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_build() {
        assert_eq!(Mode::default(), Mode::Build);
        assert!(!Mode::Build.is_plan());
        assert!(Mode::Plan.is_plan());
    }

    #[test]
    fn labels() {
        assert_eq!(Mode::Build.label(), "BUILD");
        assert_eq!(Mode::Plan.label(), "PLAN");
    }
}
