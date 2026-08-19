//! Analysis report container + severity distribution + project-key derivation.

use serde::{Deserialize, Serialize};

use crate::finding::{Finding, FindingKind, Severity};

/// Severity histogram for UI display.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SeverityDistribution {
    pub info: usize,
    pub low: usize,
    pub medium: usize,
    pub high: usize,
    pub blocker: usize,
}

impl SeverityDistribution {
    pub fn from_findings(findings: &[Finding]) -> Self {
        let mut d = Self::default();
        for f in findings {
            match f.severity {
                Severity::Info => d.info += 1,
                Severity::Low => d.low += 1,
                Severity::Medium => d.medium += 1,
                Severity::High => d.high += 1,
                Severity::Blocker => d.blocker += 1,
            }
        }
        d
    }

    pub fn total(&self) -> usize {
        self.info + self.low + self.medium + self.high + self.blocker
    }

    /// Count of findings at or above `severity` rank.
    pub fn at_or_above(&self, sev: Severity) -> usize {
        let rank = sev.rank();
        let mut n = 0;
        if Severity::Info.rank() >= rank { n += self.info; }
        if Severity::Low.rank() >= rank { n += self.low; }
        if Severity::Medium.rank() >= rank { n += self.medium; }
        if Severity::High.rank() >= rank { n += self.high; }
        if Severity::Blocker.rank() >= rank { n += self.blocker; }
        n
    }
}

/// A completed analysis run. Persisted by [`crate::store::AnalysisStore`] so
/// the controller can resume after context compaction or process restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisReport {
    /// Stable report id.
    pub id: String,
    /// Provider id that produced it (e.g. `sonarqube`).
    pub provider: String,
    /// Project the report belongs to.
    pub project: String,
    /// Absolute project root analysed.
    pub project_root: String,
    /// Timestamp (RFC3339).
    pub at: String,
    /// Optional human label.
    pub label: Option<String>,
    /// All normalised findings (sanitised).
    pub findings: Vec<Finding>,
    pub distribution: SeverityDistribution,
    /// Distinct files affected.
    pub affected_files: Vec<String>,
}

impl AnalysisReport {
    pub fn new(provider: &str, project: &str, project_root: &str, findings: Vec<Finding>) -> Self {
        let mut affected: Vec<String> = findings.iter().map(|f| f.location.path.clone()).collect();
        affected.sort();
        affected.dedup();
        let distribution = SeverityDistribution::from_findings(&findings);
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            provider: provider.into(),
            project: project.into(),
            project_root: project_root.into(),
            at: chrono::Utc::now().to_rfc3339(),
            label: None,
            findings,
            distribution,
            affected_files: affected,
        }
    }

    pub fn findings_by_severity(&self, sev: Severity) -> Vec<&Finding> {
        self.findings.iter().filter(|f| f.severity == sev).collect()
    }

    pub fn findings_by_file(&self, path: &str) -> Vec<&Finding> {
        self.findings.iter().filter(|f| f.location.path == path).collect()
    }

    pub fn findings_by_kind(&self, kind: FindingKind) -> Vec<&Finding> {
        self.findings.iter().filter(|f| f.kind == kind).collect()
    }
}

/// Derive a stable project key from a filesystem path. Mirrors how
/// SonarQube's `sonar.projectKey` is typically set when not provided.
pub fn project_key(root: &str) -> String {
    let p = std::path::Path::new(root);
    let leaf = p
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project");
    let sanitized: String = leaf
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect();
    let hash = format!("{:x}", fnv1a(root.as_bytes()));
    format!("{sanitized}-{hash}")
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::Location;

    fn f(sev: Severity, path: &str) -> Finding {
        Finding {
            id: format!("p:r:{path}:1"),
            provider: "sonarqube".into(),
            rule: "r".into(),
            severity: sev,
            kind: FindingKind::Bug,
            message: "m".into(),
            location: Location { path: path.into(), start_line: 1, end_line: 1, source_context: None },
            status: "OPEN".into(),
            project: "proj".into(),
            remediation: None,
            rule_url: None,
        }
    }

    #[test]
    fn distribution_counts() {
        let findings = vec![
            f(Severity::High, "a.ts"),
            f(Severity::High, "b.ts"),
            f(Severity::Info, "c.ts"),
            f(Severity::Blocker, "d.ts"),
        ];
        let rep = AnalysisReport::new("sonarqube", "proj", "/tmp/proj", findings);
        assert_eq!(rep.distribution.total(), 4);
        assert_eq!(rep.distribution.at_or_above(Severity::High), 3);
        assert_eq!(rep.distribution.at_or_above(Severity::Blocker), 1);
        assert_eq!(rep.affected_files.len(), 4);
    }

    #[test]
    fn project_key_stable_and_sanitized() {
        let k1 = project_key("/home/user/My Project!");
        let k2 = project_key("/home/user/My Project!");
        assert_eq!(k1, k2);
        assert!(!k1.contains(' ') && !k1.contains('!'));
        let k3 = project_key("/other/path");
        assert_ne!(k1, k3);
    }

    #[test]
    fn findings_by_file() {
        let findings = vec![f(Severity::Low, "x.rs"), f(Severity::Low, "y.rs"), f(Severity::Low, "x.rs")];
        let rep = AnalysisReport::new("p", "proj", "/r", findings);
        assert_eq!(rep.findings_by_file("x.rs").len(), 2);
        assert_eq!(rep.findings_by_file("y.rs").len(), 1);
        assert_eq!(rep.findings_by_file("z.rs").len(), 0);
    }

    #[test]
    fn report_serde_roundtrip() {
        let rep = AnalysisReport::new("p", "proj", "/r", vec![f(Severity::Medium, "a.ts")]);
        let j = serde_json::to_string(&rep).unwrap();
        let back: AnalysisReport = serde_json::from_str(&j).unwrap();
        assert_eq!(back.project, rep.project);
        assert_eq!(back.findings.len(), 1);
    }
}
