//! Persistent store of analysis reports.
//!
//! Reports are persisted per-project under `~/.aether/analysis/<project-key>/`
//! as individual JSON files. This lets the controller resume the
//! SonarQube → reason → implement → re-verify workflow after context
//! compaction, process restart, or a new session.

use std::path::PathBuf;

use serde_json;

use crate::report::AnalysisReport;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("not found: {0}")]
    NotFound(String),
}

/// Filesystem-backed store of [`AnalysisReport`]s.
#[derive(Debug, Clone)]
pub struct AnalysisStore {
    root: PathBuf,
}

impl AnalysisStore {
    /// Open (creating if needed) a store rooted at `root`.
    pub fn open(root: PathBuf) -> Result<Self, StoreError> {
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Default location: `~/.aether/analysis`.
    pub fn default_dir() -> Result<Self, StoreError> {
        let home = dirs::home_dir().ok_or_else(|| {
            StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "cannot resolve home directory",
            ))
        })?;
        Self::open(home.join(".aether").join("analysis"))
    }

    fn project_dir(&self, project: &str) -> PathBuf {
        self.root.join(project)
    }

    /// All project keys that have at least one stored report.
    pub fn projects(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for e in entries.flatten() {
                if e.path().is_dir() {
                    if let Some(name) = e.file_name().to_str() {
                        out.push(name.to_string());
                    }
                }
            }
        }
        out.sort();
        out
    }

    /// Persist a report. The report id becomes the file stem.
    pub fn save(&self, report: &AnalysisReport) -> Result<(), StoreError> {
        let dir = self.project_dir(&report.project);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.json", report.id));
        let json = serde_json::to_string_pretty(report)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load a report by project + report id.
    pub fn load(&self, project: &str, report_id: &str) -> Result<AnalysisReport, StoreError> {
        let path = self
            .project_dir(project)
            .join(format!("{}.json", report_id));
        if !path.exists() {
            return Err(StoreError::NotFound(format!("{project}/{report_id}")));
        }
        let json = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }

    /// List report ids for a project, newest first.
    pub fn list(&self, project: &str) -> Result<Vec<AnalysisReport>, StoreError> {
        let dir = self.project_dir(project);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(json) = std::fs::read_to_string(&path) {
                    if let Ok(rep) = serde_json::from_str::<AnalysisReport>(&json) {
                        out.push(rep);
                    }
                }
            }
        }
        out.sort_by(|a, b| b.at.cmp(&a.at));
        Ok(out)
    }

    /// Load the most recent report for a project, if any. Used to resume a
    /// verification loop.
    pub fn latest(&self, project: &str) -> Result<Option<AnalysisReport>, StoreError> {
        let mut all = self.list(project)?;
        all.sort_by(|a, b| b.at.cmp(&a.at));
        Ok(all.into_iter().next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::{Finding, FindingKind, Location, Severity};

    fn report(project: &str) -> AnalysisReport {
        let f = Finding {
            id: "k".into(),
            provider: "sonarqube".into(),
            rule: "r".into(),
            severity: Severity::High,
            kind: FindingKind::Bug,
            message: "m".into(),
            location: Location { path: "a.ts".into(), start_line: 1, end_line: 1, source_context: None },
            status: "OPEN".into(),
            project: project.into(),
            remediation: None,
            rule_url: None,
        };
        AnalysisReport::new("sonarqube", project, "/tmp/proj", vec![f])
    }

    fn tmp_store(name: &str) -> AnalysisStore {
        let dir = std::env::temp_dir().join(format!("aether-analysis-test-{name}-{}", uuid::Uuid::new_v4()));
        AnalysisStore::open(dir).unwrap()
    }

    #[test]
    fn save_and_load_roundtrip() {
        let store = tmp_store("roundtrip");
        let rep = report("proj");
        store.save(&rep).unwrap();
        let loaded = store.load("proj", &rep.id).unwrap();
        assert_eq!(loaded.id, rep.id);
        assert_eq!(loaded.findings.len(), 1);
        assert_eq!(loaded.project, "proj");
    }

    #[test]
    fn latest_returns_newest() {
        let store = tmp_store("latest");
        let mut older = report("proj");
        older.at = "2020-01-01T00:00:00Z".into();
        let mut newer = report("proj");
        newer.at = "2024-01-01T00:00:00Z".into();
        store.save(&older).unwrap();
        store.save(&newer).unwrap();
        let latest = store.latest("proj").unwrap().unwrap();
        assert_eq!(latest.id, newer.id);
    }

    #[test]
    fn list_is_newest_first() {
        let store = tmp_store("list");
        for i in 0..3 {
            let mut r = report("proj");
            r.at = format!("2024-01-0{i}T00:00:00Z");
            store.save(&r).unwrap();
        }
        let list = store.list("proj").unwrap();
        assert_eq!(list.len(), 3);
        assert!(list[0].at >= list[1].at);
        assert!(list[1].at >= list[2].at);
    }

    #[test]
    fn load_missing_returns_not_found() {
        let store = tmp_store("missing");
        let r = store.load("proj", "nope");
        assert!(matches!(r, Err(StoreError::NotFound(_))));
    }

    #[test]
    fn latest_on_empty_project_is_none() {
        let store = tmp_store("empty");
        assert!(store.latest("proj").unwrap().is_none());
    }
}
