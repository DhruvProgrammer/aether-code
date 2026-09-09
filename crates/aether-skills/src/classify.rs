//! Lightweight deterministic task classifier.
//!
//! No model call: keyword scoring over the request text. Deterministic and
//! reproducible — same input always yields the same kinds.

use serde::{Deserialize, Serialize};

/// Request categories used for skill retrieval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    BugFix,
    Feature,
    Refactor,
    Architecture,
    Research,
    Security,
    Database,
    Api,
    Documentation,
    Testing,
    Refactor2,
    Provider,
    Workspace,
    Context,
    Compaction,
    Unknown,
}

impl TaskKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskKind::BugFix => "bug_fix",
            TaskKind::Feature => "feature",
            TaskKind::Refactor | TaskKind::Refactor2 => "refactor",
            TaskKind::Architecture => "architecture",
            TaskKind::Research => "research",
            TaskKind::Security => "security",
            TaskKind::Database => "database",
            TaskKind::Api => "api",
            TaskKind::Documentation => "documentation",
            TaskKind::Testing => "testing",
            TaskKind::Provider => "provider",
            TaskKind::Workspace => "workspace",
            TaskKind::Context => "context",
            TaskKind::Compaction => "compaction",
            TaskKind::Unknown => "unknown",
        }
    }
}

fn score(text: &str, words: &[&str]) -> u32 {
    let lower = text.to_lowercase();
    words.iter().map(|w| lower.matches(w).count() as u32).sum()
}

/// Classify a request. Returns kinds sorted by score (best first); empty
/// means Unknown. Deterministic: ties broken by fixed order below.
pub fn classify(request: &str) -> Vec<TaskKind> {
    let table: &[(TaskKind, &[&str])] = &[
        (TaskKind::BugFix, &["fix", "bug", "error", "broken", "crash", "fail", "regression", "issue"]),
        (TaskKind::Security, &["security", "auth", "vulnerab", "secret", "permission", "injection", "audit", "xss", "csrf", "ssrf"]),
        (TaskKind::Database, &["database", "schema", "migration", "sql", "table", "postgres", "sqlite", "index"]),
        (TaskKind::Api, &["api", "endpoint", "rest", "route", "contract", "openapi", "webhook"]),
        (TaskKind::Testing, &["test", "testing", "coverage", "pytest", "cargo test", "vitest", "e2e"]),
        (TaskKind::Documentation, &["doc", "readme", "changelog", "comment", "guide"]),
        (TaskKind::Architecture, &["architect", "design doc", "system design", "adr", "component"]),
        (TaskKind::Research, &["research", "investigate", "explore", "spike", "evaluate", "compare"]),
        (TaskKind::Refactor, &["refactor", "cleanup", "restructure", "simplify", "rename"]),
        (TaskKind::Provider, &["provider", "model", "api key", "endpoint", "openai", "nvidia", "ollama"]),
        (TaskKind::Compaction, &["compact", "context limit", "context window", "summar", "checkpoint"]),
        (TaskKind::Context, &["context", "prompt", "token", "budget"]),
        (TaskKind::Workspace, &["workspace", "folder", "file", "directory", "repo"]),
        (TaskKind::Feature, &["add", "feature", "implement", "create", "build", "new"]),
    ];
    let mut scored: Vec<(TaskKind, u32)> = table
        .iter()
        .map(|(k, words)| (*k, score(request, words)))
        .filter(|(_, s)| *s > 0)
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.into_iter().map(|(k, _)| k).collect()
}

/// Map task kinds to preferred skill section ids (in priority order).
pub fn kinds_to_sections(kinds: &[TaskKind]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    let mut push = |s: &'static str| {
        if !out.contains(&s) {
            out.push(s);
        }
    };
    for k in kinds {
        match k {
            TaskKind::BugFix => {
                push("existing-project");
                push("implementation");
                push("testing");
                push("errors");
            }
            TaskKind::Feature => {
                push("requirements");
                push("implementation");
                push("testing");
                push("acceptance");
                push("tasks");
            }
            TaskKind::Refactor => {
                push("refactoring");
                push("testing");
                push("existing-project");
            }
            TaskKind::Architecture => {
                push("architecture");
                push("technical-design");
                push("requirements");
                push("adr");
            }
            TaskKind::Research => {
                push("research");
                push("reconnaissance");
            }
            TaskKind::Security => {
                push("security");
                push("testing");
                push("implementation");
            }
            TaskKind::Database => {
                push("database");
                push("testing");
                push("architecture");
            }
            TaskKind::Api => {
                push("api");
                push("testing");
                push("technical-design");
            }
            TaskKind::Documentation => {
                push("documentation");
            }
            TaskKind::Testing => {
                push("testing");
                push("acceptance");
            }
            TaskKind::Provider => {
                push("implementation");
                push("errors");
            }
            TaskKind::Workspace => {
                push("reconnaissance");
                push("existing-project");
            }
            TaskKind::Context | TaskKind::Compaction => {
                push("implementation");
                push("errors");
            }
            TaskKind::Refactor2 | TaskKind::Unknown => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_bug_fix() {
        let kinds = classify("Fix an authentication bug that crashes on login");
        assert!(kinds.contains(&TaskKind::BugFix));
        assert!(kinds.contains(&TaskKind::Security));
    }

    #[test]
    fn classifies_database_schema() {
        let kinds = classify("Design a new database schema with migrations");
        assert!(kinds.contains(&TaskKind::Database));
    }

    #[test]
    fn deterministic_same_input_same_output() {
        let a = classify("add feature with tests");
        let b = classify("add feature with tests");
        assert_eq!(a, b);
    }

    #[test]
    fn sections_cover_bug_fix() {
        let kinds = classify("Fix an authentication bug");
        let sections = kinds_to_sections(&kinds);
        assert!(sections.contains(&"testing"));
        assert!(sections.contains(&"security"));
        assert!(sections.contains(&"implementation"));
        // Must NOT include unrelated sections
        assert!(!sections.contains(&"database"));
        assert!(!sections.contains(&"adr"));
        assert!(!sections.contains(&"observability"));
    }
}
