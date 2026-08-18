//! Task classification — given a free-form task description, infer what kind
//! of task it is and which capabilities it requires.

use serde::{Deserialize, Serialize};

use super::capability::Capability;

/// Classified task kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// Implement / write / fix / refactor code.
    Code,
    /// Read-only review or audit.
    Review,
    /// Research / investigation / reading.
    Research,
    /// Plan / design / propose approach.
    Plan,
    /// Summarise / explain / describe.
    Summarize,
    /// Security audit / threat model.
    Security,
    /// Test authoring / test running.
    Test,
    /// Visual / UI / frontend work.
    Visual,
    /// Generic / unspecified — fallback category.
    Generic,
}

/// Signals inferred from the task description and context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSignals {
    pub kind: TaskKind,
    /// Required capabilities the chosen model must have.
    pub required_capabilities: Vec<Capability>,
    /// Heuristic complexity estimate (0 = trivial, 1 = very complex).
    pub complexity: f32,
    /// Estimated context size (in tokens) the task will need.
    pub estimated_context_tokens: u32,
}

impl TaskSignals {
    pub fn classify(task: &str, context_chars: usize) -> Self {
        let lower = task.to_ascii_lowercase();
        let (kind, caps) = classify_kind_and_caps(&lower);
        let complexity = estimate_complexity(task, kind);
        let est_tokens = estimate_context_tokens(task, context_chars);
        Self {
            kind,
            required_capabilities: caps,
            complexity,
            estimated_context_tokens: est_tokens,
        }
    }
}

fn classify_kind_and_caps(lower: &str) -> (TaskKind, Vec<Capability>) {
    // Highest specificity first: visual > security > test > review > ...
    let visual = lower.contains("css")
        || lower.contains("tailwind")
        || lower.contains("react component")
        || lower.contains("ui")
        || lower.contains("frontend")
        || lower.contains("button")
        || lower.contains("layout");
    if visual {
        return (
            TaskKind::Visual,
            vec![Capability::ToolCalling, Capability::Vision],
        );
    }
    let security = lower.contains("security")
        || lower.contains("vulnerability")
        || lower.contains("exploit")
        || lower.contains("cve")
        || lower.contains("threat model");
    if security {
        return (
            TaskKind::Security,
            vec![Capability::ToolCalling, Capability::Reasoning],
        );
    }
    let test = lower.contains("test")
        || lower.contains("unittest")
        || lower.contains("integration test")
        || lower.contains("jest")
        || lower.contains("pytest");
    if test && (lower.contains("run") || lower.contains("execute") || lower.contains("verify")) {
        return (
            TaskKind::Test,
            vec![Capability::ToolCalling, Capability::Streaming],
        );
    }
    let review = lower.contains("review")
        || lower.contains("audit")
        || lower.contains("inspect")
        || lower.contains("check the diff");
    if review && !lower.contains("implement") && !lower.contains("fix") {
        return (TaskKind::Review, vec![Capability::ToolCalling]);
    }
    let plan = lower.contains("plan ")
        || lower.starts_with("plan")
        || lower.contains("design ")
        || lower.contains("propose")
        || lower.contains("outline approach");
    if plan && !lower.contains("implement") {
        return (
            TaskKind::Plan,
            vec![Capability::Reasoning, Capability::StructuredOutput],
        );
    }
    let research = lower.contains("research")
        || lower.contains("investigate")
        || lower.contains("look up")
        || lower.contains("read the");
    if research && !lower.contains("implement") && !lower.contains("fix") {
        return (TaskKind::Research, vec![Capability::ToolCalling]);
    }
    let summarize = lower.contains("explain")
        || lower.contains("summarise")
        || lower.contains("summarize")
        || lower.contains("describe ")
        || lower.starts_with("what is")
        || lower.starts_with("why ")
        || lower.starts_with("how does");
    if summarize && !lower.contains("implement") && !lower.contains("fix") {
        return (TaskKind::Summarize, vec![Capability::Streaming]);
    }
    let code = lower.contains("implement")
        || lower.contains("fix")
        || lower.contains("refactor")
        || lower.contains("write ")
        || lower.contains("create ")
        || lower.contains("add ")
        || lower.contains("build ");
    if code {
        return (
            TaskKind::Code,
            vec![Capability::ToolCalling, Capability::Streaming],
        );
    }
    (TaskKind::Generic, vec![Capability::Streaming])
}

fn estimate_complexity(task: &str, kind: TaskKind) -> f32 {
    let chars = task.chars().count() as f32;
    let len_factor = (chars / 2000.0).min(1.0);
    let kind_factor = match kind {
        TaskKind::Generic => 0.1,
        TaskKind::Summarize => 0.2,
        TaskKind::Research => 0.4,
        TaskKind::Plan => 0.6,
        TaskKind::Review => 0.5,
        TaskKind::Test => 0.6,
        TaskKind::Security => 0.8,
        TaskKind::Code => 0.7,
        TaskKind::Visual => 0.7,
    };
    (len_factor * 0.4 + kind_factor * 0.6).clamp(0.0, 1.0)
}

fn estimate_context_tokens(task: &str, context_chars: usize) -> u32 {
    // Rough heuristic: 4 chars ≈ 1 token for English.
    let total = task.chars().count() + context_chars;
    ((total / 4) as u32).max(64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_code() {
        let s = TaskSignals::classify("implement the OAuth login flow", 0);
        assert_eq!(s.kind, TaskKind::Code);
        assert!(s.required_capabilities.contains(&Capability::ToolCalling));
    }

    #[test]
    fn classifies_security() {
        let s = TaskSignals::classify("audit the codebase for security vulnerabilities", 0);
        assert_eq!(s.kind, TaskKind::Security);
        assert!(s.required_capabilities.contains(&Capability::Reasoning));
    }

    #[test]
    fn classifies_visual() {
        let s = TaskSignals::classify("add a centered tailwind button to the landing page", 0);
        assert_eq!(s.kind, TaskKind::Visual);
    }

    #[test]
    fn classify_summarize() {
        let s = TaskSignals::classify("explain how the loop works", 0);
        assert_eq!(s.kind, TaskKind::Summarize);
    }

    #[test]
    fn classify_plan() {
        let s = TaskSignals::classify("plan the migration to v2", 0);
        assert_eq!(s.kind, TaskKind::Plan);
        assert!(s.required_capabilities.contains(&Capability::Reasoning));
    }

    #[test]
    fn classify_test_run() {
        let s = TaskSignals::classify("run the integration tests", 0);
        assert_eq!(s.kind, TaskKind::Test);
    }

    #[test]
    fn complexity_higher_for_long_security() {
        let a = TaskSignals::classify("audit security vulnerabilities", 0);
        let b = TaskSignals::classify("audit security vulnerabilities in the entire codebase, examine each module, document all CVE-class issues, produce threat model", 0);
        assert!(b.complexity > a.complexity);
    }
}
