//! Compare two analysis runs: resolved / remaining / new / regressed.
//!
//! The controller uses this after a fix cycle to decide whether another
//! implementation round is needed — findings that disappeared are resolved,
//! findings that appear for the first time are newly introduced, and findings
//! that stayed the same are remaining. Severity regressions are called out
//! explicitly.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::finding::{Finding, Severity};

/// Result of diffing a newer analysis against an older baseline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisDiff {
    pub baseline_report: String,
    pub current_report: String,
    /// Fingerprints present in the baseline but absent now.
    pub resolved: Vec<String>,
    /// Fingerprints present in both runs.
    pub remaining: Vec<String>,
    /// Fingerprints present now but absent from the baseline.
    pub introduced: Vec<String>,
    /// Findings whose severity increased between runs (fingerprint → old, new).
    pub regressions: Vec<(String, Severity, Severity)>,
    pub baseline_count: usize,
    pub current_count: usize,
}

impl AnalysisDiff {
    pub fn resolved_count(&self) -> usize {
        self.resolved.len()
    }

    pub fn introduced_count(&self) -> usize {
        self.introduced.len()
    }

    /// True when the run strictly improved quality: nothing introduced and at
    /// least one finding resolved (or an empty current set with a non-empty
    /// baseline).
    pub fn is_improvement(&self) -> bool {
        self.regressions.is_empty() && self.introduced.is_empty() && self.resolved_count() > 0
    }

    /// True when nothing changed between runs.
    pub fn is_unchanged(&self) -> bool {
        self.resolved.is_empty() && self.introduced.is_empty() && self.regressions.is_empty()
    }

    /// Compact render for controller context.
    pub fn render(&self) -> String {
        let mut out = format!(
            "Analysis diff: {} → {} findings ({} resolved, {} new, {} regressions)\n",
            self.baseline_count,
            self.current_count,
            self.resolved_count(),
            self.introduced_count(),
            self.regressions.len(),
        );
        for (fp, old, new) in &self.regressions {
            out.push_str(&format!("  REGRESSION {fp}: {} → {}\n", old, new));
        }
        if !self.is_improvement() && self.introduced_count() > 0 {
            out.push_str("  WARNING: new findings were introduced by recent changes\n");
        }
        out
    }
}

/// Diff `current` against `baseline`. Findings are matched by fingerprint
/// (provider+rule+file+line), which is stable across re-scans of unchanged
/// code.
pub fn diff(baseline: &[Finding], current: &[Finding], baseline_report: &str, current_report: &str) -> AnalysisDiff {
    let base_map: HashMap<String, Severity> = baseline
        .iter()
        .map(|f| (f.fingerprint(), f.severity))
        .collect();
    let cur_map: HashMap<String, Severity> = current
        .iter()
        .map(|f| (f.fingerprint(), f.severity))
        .collect();

    let mut resolved = Vec::new();
    for fp in base_map.keys() {
        if !cur_map.contains_key(fp) {
            resolved.push(fp.clone());
        }
    }
    resolved.sort();

    let mut remaining = Vec::new();
    let mut regressions = Vec::new();
    for (fp, sev) in &cur_map {
        match base_map.get(fp) {
            Some(old) => {
                remaining.push(fp.clone());
                if sev.rank() > old.rank() {
                    regressions.push((fp.clone(), *old, *sev));
                }
            }
            None => {}
        }
    }
    remaining.sort();
    regressions.sort_by(|a, b| a.0.cmp(&b.0));

    let mut introduced = Vec::new();
    for fp in cur_map.keys() {
        if !base_map.contains_key(fp) {
            introduced.push(fp.clone());
        }
    }
    introduced.sort();

    AnalysisDiff {
        baseline_report: baseline_report.into(),
        current_report: current_report.into(),
        resolved,
        remaining,
        introduced,
        regressions,
        baseline_count: baseline.len(),
        current_count: current.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::{FindingKind, Location};

    fn f(rule: &str, path: &str, line: u32, sev: Severity) -> Finding {
        Finding {
            id: format!("k-{rule}-{path}-{line}"),
            provider: "sonarqube".into(),
            rule: rule.into(),
            severity: sev,
            kind: FindingKind::CodeSmell,
            message: "m".into(),
            location: Location { path: path.into(), start_line: line, end_line: line, source_context: None },
            status: "OPEN".into(),
            project: "p".into(),
            remediation: None,
            rule_url: None,
        }
    }

    #[test]
    fn resolved_remaining_introduced() {
        let baseline = vec![
            f("S1", "a.ts", 10, Severity::High),
            f("S2", "b.ts", 20, Severity::Medium),
        ];
        let current = vec![
            f("S2", "b.ts", 20, Severity::Medium),
            f("S3", "c.ts", 30, Severity::Low),
        ];
        let d = diff(&baseline, &current, "r1", "r2");
        assert_eq!(d.resolved.len(), 1);
        assert!(d.resolved[0].contains("S1"));
        assert_eq!(d.remaining.len(), 1);
        assert_eq!(d.introduced.len(), 1);
        assert!(d.introduced[0].contains("S3"));
        assert!(!d.is_improvement()); // new finding introduced
        assert!(!d.is_unchanged());
    }

    #[test]
    fn pure_improvement() {
        let baseline = vec![f("S1", "a.ts", 10, Severity::High)];
        let current: Vec<Finding> = vec![];
        let d = diff(&baseline, &current, "r1", "r2");
        assert!(d.is_improvement());
        assert_eq!(d.resolved_count(), 1);
        assert_eq!(d.introduced_count(), 0);
    }

    #[test]
    fn regression_detection() {
        let baseline = vec![f("S1", "a.ts", 10, Severity::Low)];
        let current = vec![f("S1", "a.ts", 10, Severity::Blocker)];
        let d = diff(&baseline, &current, "r1", "r2");
        assert_eq!(d.regressions.len(), 1);
        assert_eq!(d.regressions[0].1, Severity::Low);
        assert_eq!(d.regressions[0].2, Severity::Blocker);
        assert!(d.regressions[0].0.contains("S1"));
        assert!(!d.is_improvement());
    }

    #[test]
    fn unchanged_detection() {
        let baseline = vec![f("S1", "a.ts", 10, Severity::Low)];
        let current = vec![f("S1", "a.ts", 10, Severity::Low)];
        let d = diff(&baseline, &current, "r1", "r2");
        assert!(d.is_unchanged());
        assert_eq!(d.remaining.len(), 1);
    }

    #[test]
    fn render_mentions_counts() {
        let baseline = vec![f("S1", "a.ts", 10, Severity::Low)];
        let current = vec![f("S1", "a.ts", 10, Severity::High)];
        let d = diff(&baseline, &current, "r1", "r2");
        let text = d.render();
        assert!(text.contains("1 → 1 findings"));
        assert!(text.contains("REGRESSION"));
    }
}
