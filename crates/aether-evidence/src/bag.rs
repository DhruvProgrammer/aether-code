//! `EvidenceBag` — collected evidence from one or more subagents.

use std::collections::HashMap;

use parking_lot::RwLock;

use crate::evidence::{Evidence, EvidenceId};

/// Collection of `Evidence` records grouped by source agent. Thread-safe.
pub struct EvidenceBag {
    inner: RwLock<HashMap<EvidenceId, Evidence>>,
    by_agent: RwLock<HashMap<String, Vec<EvidenceId>>>,
}

impl Default for EvidenceBag {
    fn default() -> Self {
        Self::new()
    }
}

impl EvidenceBag {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            by_agent: RwLock::new(HashMap::new()),
        }
    }

    /// Add an evidence record to the bag.
    pub fn add(&self, e: Evidence) -> EvidenceId {
        let id = e.id.clone();
        let agent = e.source_agent.clone();
        self.inner.write().insert(id.clone(), e);
        self.by_agent.write().entry(agent).or_default().push(id.clone());
        id
    }

    /// Look up evidence by id.
    pub fn get(&self, id: &EvidenceId) -> Option<Evidence> {
        self.inner.read().get(id).cloned()
    }

    /// All evidence records emitted by a specific agent.
    pub fn for_agent(&self, agent: &str) -> Vec<Evidence> {
        let ids = self
            .by_agent
            .read()
            .get(agent)
            .cloned()
            .unwrap_or_default();
        self.inner
            .read()
            .iter()
            .filter(|(k, _)| ids.contains(k))
            .map(|(_, v)| v.clone())
            .collect()
    }

    /// All evidence records, in insertion order (by id).
    pub fn all(&self) -> Vec<Evidence> {
        self.inner.read().values().cloned().collect()
    }

    /// Total number of evidence records.
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    /// Is the bag empty?
    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }

    /// Sum of confidence values, weighted by test_pass count (more tests
    /// = more weight). 0.0 if empty.
    pub fn aggregate_confidence(&self) -> f32 {
        let all = self.all();
        if all.is_empty() {
            return 0.0;
        }
        let mut total = 0.0f32;
        let mut weight = 0.0f32;
        for e in &all {
            let w = 1.0 + e.tests_passed() as f32 * 0.1;
            total += e.confidence.0 * w;
            weight += w;
        }
        if weight > 0.0 {
            total / weight
        } else {
            0.0
        }
    }

    /// Count of evidence records with at least one critical contradiction.
    pub fn critical_contradictions(&self) -> usize {
        self.all().iter().filter(|e| e.has_critical_contradiction()).count()
    }

    /// Collect every distinct file path mentioned across all evidence.
    pub fn all_files(&self) -> Vec<std::path::PathBuf> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for e in self.all() {
            for f in e.files {
                if seen.insert(f.clone()) {
                    out.push(f);
                }
            }
        }
        out
    }

    /// Distinct agent ids that contributed evidence.
    pub fn contributing_agents(&self) -> Vec<String> {
        let mut v: Vec<String> = self.by_agent.read().keys().cloned().collect();
        v.sort();
        v
    }

    /// Clear all evidence (e.g. for a new task cycle).
    pub fn clear(&self) {
        self.inner.write().clear();
        self.by_agent.write().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{Evidence, EvidenceKind, Recommendation, TestRef};
    use std::path::PathBuf;

    fn ev(agent: &str, claim: &str, conf: f32, tests: usize) -> Evidence {
        let mut e = Evidence::new(agent, "implementer", EvidenceKind::Implementation, claim)
            .with_confidence(crate::evidence::Confidence(conf))
            .with_recommendation(Recommendation::Pass);
        for i in 0..tests {
            e = e.with_test(TestRef {
                path: PathBuf::from(format!("test_{i}.rs")),
                name: format!("t{i}"),
                passed: true,
                duration_ms: Some(10),
            });
        }
        e
    }

    #[test]
    fn add_and_lookup() {
        let bag = EvidenceBag::new();
        let e = ev("coder", "added login", 0.9, 2);
        let id = bag.add(e);
        assert!(bag.get(&id).is_some());
        assert_eq!(bag.len(), 1);
    }

    #[test]
    fn for_agent_filters() {
        let bag = EvidenceBag::new();
        bag.add(ev("coder", "x", 0.5, 0));
        bag.add(ev("tester", "y", 0.8, 1));
        bag.add(ev("coder", "z", 0.7, 3));
        assert_eq!(bag.for_agent("coder").len(), 2);
        assert_eq!(bag.for_agent("tester").len(), 1);
        assert_eq!(bag.for_agent("reviewer").len(), 0);
    }

    #[test]
    fn aggregate_confidence_weights_tests() {
        let bag = EvidenceBag::new();
        bag.add(ev("a", "x", 0.5, 0));
        bag.add(ev("b", "y", 0.9, 10));
        let c = bag.aggregate_confidence();
        // Weighted average: b has more tests → should dominate.
        assert!(c > 0.7);
    }

    #[test]
    fn contributing_agents_sorted() {
        let bag = EvidenceBag::new();
        bag.add(ev("z", "x", 0.5, 0));
        bag.add(ev("a", "y", 0.8, 0));
        assert_eq!(bag.contributing_agents(), vec!["a", "z"]);
    }

    #[test]
    fn all_files_dedupes() {
        let bag = EvidenceBag::new();
        let mut e1 = ev("a", "x", 0.5, 0);
        e1 = e1.with_file("src/auth.rs").with_file("src/main.rs");
        let mut e2 = ev("b", "y", 0.7, 0);
        e2 = e2.with_file("src/auth.rs");
        bag.add(e1);
        bag.add(e2);
        let files = bag.all_files();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn clear_empties() {
        let bag = EvidenceBag::new();
        bag.add(ev("a", "x", 0.5, 0));
        bag.clear();
        assert!(bag.is_empty());
    }
}
