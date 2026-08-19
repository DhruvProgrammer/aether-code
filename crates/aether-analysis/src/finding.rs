//! Normalised internal finding format.
//!
//! Every analyzer (SonarQube today, ESLint/Semgrep later) is mapped into this
//! canonical schema so the controller reasons over one uniform structure and
//! no analyzer-specific internals leak into prompts.

use serde::{Deserialize, Serialize};

/// Finding severity, mapped from each provider's native scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Blocker,
}

impl Severity {
    /// Rank for sorting (higher = worse).
    pub fn rank(self) -> u8 {
        match self {
            Severity::Info => 0,
            Severity::Low => 1,
            Severity::Medium => 2,
            Severity::High => 3,
            Severity::Blocker => 4,
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Severity::Info => "info",
            Severity::Low => "low",
            Severity::Medium => "medium",
            Severity::High => "high",
            Severity::Blocker => "blocker",
        };
        f.write_str(s)
    }
}

/// Issue category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// Correctness bug.
    Bug,
    /// Security weakness.
    Vulnerability,
    /// Hot security-sensitive code needing review.
    SecurityHotspot,
    /// Maintainability issue.
    CodeSmell,
}

impl std::fmt::Display for FindingKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            FindingKind::Bug => "bug",
            FindingKind::Vulnerability => "vulnerability",
            FindingKind::SecurityHotspot => "security_hotspot",
            FindingKind::CodeSmell => "code_smell",
        };
        f.write_str(s)
    }
}

/// A source location within the analysed project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    /// Path relative to the analysed project root.
    pub path: String,
    /// 1-based start line (0 when unknown).
    pub start_line: u32,
    /// 1-based end line (defaults to `start_line` when unknown).
    pub end_line: u32,
    /// Optional surrounding source lines for context (never includes secrets;
    /// sanitised before storage).
    pub source_context: Option<String>,
}

/// One normalised analysis finding. This is what the controller sees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// Stable id within a report (`<provider>:<rule>:<path>:<line>`).
    pub id: String,
    /// Provider id, e.g. `sonarqube`, `eslint`, `semgrep`.
    pub provider: String,
    /// Analyzer rule identifier, e.g. `typescript:S3776`.
    pub rule: String,
    pub severity: Severity,
    pub kind: FindingKind,
    /// Human-readable message.
    pub message: String,
    pub location: Location,
    /// Status reported by the analyzer, e.g. `OPEN`, `CONFIRMED`, `RESOLVED`.
    pub status: String,
    /// Project/component the finding belongs to.
    pub project: String,
    /// Estimated effort to fix, human readable, e.g. `"15min"`.
    pub remediation: Option<String>,
    /// Optional URL/anchor to the rule documentation.
    pub rule_url: Option<String>,
}

impl Finding {
    /// Dedup key — the same underlying issue across repeated scans.
    pub fn fingerprint(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.provider, self.rule, self.location.path, self.location.start_line,
        )
    }

    /// Short one-line render for controller context (compact on purpose).
    pub fn render_line(&self) -> String {
        format!(
            "[{}][{}] {} {}:{} — {}",
            self.severity, self.kind, self.rule, self.location.path, self.location.start_line, self.message,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(rule: &str, sev: Severity) -> Finding {
        Finding {
            id: format!("sonarqube:{rule}:src/a.ts:1"),
            provider: "sonarqube".into(),
            rule: rule.into(),
            severity: sev,
            kind: FindingKind::CodeSmell,
            message: "msg".into(),
            location: Location {
                path: "src/a.ts".into(),
                start_line: 1,
                end_line: 1,
                source_context: None,
            },
            status: "OPEN".into(),
            project: "proj".into(),
            remediation: None,
            rule_url: None,
        }
    }

    #[test]
    fn severity_ordering() {
        assert!(Severity::Blocker > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert_eq!(Severity::Info.rank(), 0);
        assert_eq!(Severity::Blocker.rank(), 4);
    }

    #[test]
    fn fingerprint_is_stable() {
        let a = mk("S1", Severity::Low);
        let b = mk("S1", Severity::Low);
        assert_eq!(a.fingerprint(), b.fingerprint());
        let c = mk("S2", Severity::Low);
        assert_ne!(a.fingerprint(), c.fingerprint());
    }

    #[test]
    fn render_line_is_compact() {
        let f = mk("S3776", Severity::High);
        let line = f.render_line();
        assert!(line.contains("S3776"));
        assert!(line.contains("src/a.ts:1"));
    }

    #[test]
    fn serde_roundtrip() {
        let f = mk("S1", Severity::Medium);
        let j = serde_json::to_string(&f).unwrap();
        let back: Finding = serde_json::from_str(&j).unwrap();
        assert_eq!(back, f);
    }
}
