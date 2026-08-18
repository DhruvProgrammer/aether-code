//! The `Evidence` record and its sub-types.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A unique identifier for an evidence record.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EvidenceId(pub String);

impl EvidenceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for EvidenceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for EvidenceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Confidence score, 0.0 (no confidence) to 1.0 (fully verified).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Confidence(pub f32);

impl Confidence {
    pub fn zero() -> Self {
        Self(0.0)
    }
    pub fn one() -> Self {
        Self(1.0)
    }
    pub fn is_high(&self) -> bool {
        self.0 >= 0.85
    }
    pub fn is_medium(&self) -> bool {
        (0.5..0.85).contains(&self.0)
    }
    pub fn is_low(&self) -> bool {
        self.0 < 0.5
    }
}

impl Default for Confidence {
    fn default() -> Self {
        Self(0.5)
    }
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.2}", self.0)
    }
}

/// Reference to a tool execution (e.g. "ran `cargo test`, exit 0").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultRef {
    pub tool: String,
    pub summary: String,
    pub exit_code: Option<i32>,
    pub stderr_excerpt: Option<String>,
}

/// Reference to a test (path + name + result).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRef {
    pub path: PathBuf,
    pub name: String,
    pub passed: bool,
    pub duration_ms: Option<u64>,
}

/// An explicit caveat or contradiction the agent flags in its own claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contradiction {
    pub description: String,
    pub source: String,
    pub severity: ContradictionSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContradictionSeverity {
    /// Informational — the agent is aware but considers it not material.
    Info,
    /// Material — the agent considers this a real concern.
    Material,
    /// Critical — the agent believes the claim may be invalidated.
    Critical,
}

/// The recommendation the agent attaches to its evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recommendation {
    /// The agent believes the work is correct and ready for the controller
    /// to accept.
    Pass,
    /// The agent believes the controller should re-plan with the new info.
    Replan,
    /// The agent believes a human must review before acceptance.
    ManualReview,
    /// The agent is uncertain and cannot recommend.
    Uncertain,
}

/// What kind of evidence this is — used for routing and UI display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// "I implemented X"
    Implementation,
    /// "I tested X and observed Y"
    Test,
    /// "I reviewed X and found Y"
    Review,
    /// "I researched X and learned Y"
    Research,
    /// "I planned X"
    Plan,
    /// "I audited X for security and found Y"
    Security,
    /// "I refactored X"
    Refactor,
    /// "I documented X"
    Documentation,
    /// "I made a free-form observation"
    Observation,
}

/// A single piece of structured evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: EvidenceId,
    pub source_agent: String,
    pub role: String,
    pub kind: EvidenceKind,
    pub claim: String,
    pub confidence: Confidence,
    pub files: Vec<PathBuf>,
    pub tool_results: Vec<ToolResultRef>,
    pub tests: Vec<TestRef>,
    pub contradictions: Vec<Contradiction>,
    pub recommendation: Recommendation,
    pub summary: String,
    pub created_at: DateTime<Utc>,
}

impl Evidence {
    pub fn new(source_agent: impl Into<String>, role: impl Into<String>, kind: EvidenceKind, claim: impl Into<String>) -> Self {
        Self {
            id: EvidenceId::new(),
            source_agent: source_agent.into(),
            role: role.into(),
            kind,
            claim: claim.into(),
            confidence: Confidence::default(),
            files: Vec::new(),
            tool_results: Vec::new(),
            tests: Vec::new(),
            contradictions: Vec::new(),
            recommendation: Recommendation::Pass,
            summary: String::new(),
            created_at: Utc::now(),
        }
    }

    pub fn with_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.files.push(path.into());
        self
    }

    pub fn with_files(mut self, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        self.files.extend(paths);
        self
    }

    pub fn with_tool_result(mut self, t: ToolResultRef) -> Self {
        self.tool_results.push(t);
        self
    }

    pub fn with_test(mut self, t: TestRef) -> Self {
        self.tests.push(t);
        self
    }

    pub fn with_contradiction(mut self, c: Contradiction) -> Self {
        self.contradictions.push(c);
        self
    }

    pub fn with_recommendation(mut self, r: Recommendation) -> Self {
        self.recommendation = r;
        self
    }

    pub fn with_confidence(mut self, c: Confidence) -> Self {
        self.confidence = c;
        self
    }

    pub fn with_summary(mut self, s: impl Into<String>) -> Self {
        self.summary = s.into();
        self
    }

    /// Quick flag: does this evidence contain any critical contradiction?
    pub fn has_critical_contradiction(&self) -> bool {
        self.contradictions
            .iter()
            .any(|c| c.severity == ContradictionSeverity::Critical)
    }

    /// Count of test results that passed.
    pub fn tests_passed(&self) -> usize {
        self.tests.iter().filter(|t| t.passed).count()
    }

    /// Count of test results that failed.
    pub fn tests_failed(&self) -> usize {
        self.tests.iter().filter(|t| !t.passed).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_builder_constructs_correctly() {
        let e = Evidence::new("coder", "implementer", EvidenceKind::Implementation, "Added OAuth")
            .with_file("src/auth.rs")
            .with_test(TestRef {
                path: "tests/auth_test.rs".into(),
                name: "login".into(),
                passed: true,
                duration_ms: Some(120),
            })
            .with_confidence(Confidence(0.9))
            .with_recommendation(Recommendation::Pass);
        assert_eq!(e.files.len(), 1);
        assert_eq!(e.tests.len(), 1);
        assert_eq!(e.confidence.0, 0.9);
        assert!(e.confidence.is_high());
        assert_eq!(e.tests_passed(), 1);
        assert_eq!(e.tests_failed(), 0);
        assert!(!e.has_critical_contradiction());
    }

    #[test]
    fn critical_contradiction_is_flagged() {
        let e = Evidence::new("coder", "implementer", EvidenceKind::Implementation, "Added OAuth")
            .with_contradiction(Contradiction {
                description: "Test suite not actually run".into(),
                source: "self".into(),
                severity: ContradictionSeverity::Critical,
            });
        assert!(e.has_critical_contradiction());
    }
}
