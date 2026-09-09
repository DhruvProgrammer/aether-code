//! Skill retriever: deterministic section selection.
//!
//! Selection sources (merged, deduped, dependencies resolved):
//! 1. `always: true` sections (core, context)
//! 2. explicit `@skill/section` references in the request
//! 3. keyword scoring over section keywords + description
//! 4. task-kind mapping (`classify::kinds_to_sections`)

use std::collections::{HashMap, HashSet};

use crate::classify::{classify, kinds_to_sections};
use crate::index::{SectionMeta, SkillIndex};

/// A selected section with its resolved text.
#[derive(Debug, Clone)]
pub struct RetrievedSection {
    pub id: String,
    pub text: String,
    pub tokens: u32,
    pub from_explicit_ref: bool,
}

pub struct SkillRetriever<'a> {
    index: &'a SkillIndex,
}

impl<'a> SkillRetriever<'a> {
    pub fn new(index: &'a SkillIndex) -> Self {
        Self { index }
    }

    /// Extract explicit `@skill-id/section-id` or `@section-id` references.
    pub fn explicit_refs(request: &str) -> Vec<String> {
        let mut out = Vec::new();
        for token in request.split_whitespace() {
            if let Some(rest) = token.strip_prefix('@') {
                let clean = rest.trim_matches(|c: char| {
                    c == ',' || c == '.' || c == ')' || c == '(' || c == '"' || c == '\''
                });
                // Accept `software-engineering/testing` or bare `testing`
                let section = clean.rsplit('/').next().unwrap_or(clean);
                if !section.is_empty() && section.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    out.push(section.to_string());
                }
            }
        }
        out
    }

    fn keyword_score(request: &str, meta: &SectionMeta) -> u32 {
        let lower = request.to_lowercase();
        let mut s = 0u32;
        for kw in &meta.keywords {
            let k = kw.to_lowercase();
            s += lower.matches(k.as_str()).count() as u32;
        }
        // Description words carry half weight via single-word hits
        for w in meta.description.split(|c: char| !c.is_alphanumeric()) {
            if w.len() > 4 && lower.contains(&w.to_lowercase()) {
                s += 1;
            }
        }
        // Priority as tiebreak weight (scaled down)
        s * 10 + meta.priority.min(9)
    }

    /// Retrieve sections for a request.
    ///
    /// * `max_sections` caps non-always sections (dependencies still resolve).
    /// * `compact` selects `.compact.md` when available (lower token cost).
    pub fn retrieve(&self, request: &str, max_sections: usize, compact: bool) -> Vec<RetrievedSection> {
        let mut selected: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut explicit: HashSet<String> = HashSet::new();

        // 1. always-load sections first (stable order)
        for meta in self.index.sections() {
            if meta.always && seen.insert(meta.id.clone()) {
                selected.push(meta.id.clone());
            }
        }

        // 2. explicit @refs
        for r in Self::explicit_refs(request) {
            if self.index.get(&r).is_some() && seen.insert(r.clone()) {
                selected.push(r.clone());
                explicit.insert(r);
            }
        }

        // 3. task-kind mapping
        let kinds = classify(request);
        for s in kinds_to_sections(&kinds) {
            if self.index.get(s).is_some() && seen.insert(s.to_string()) {
                selected.push(s.to_string());
            }
        }

        // 4. keyword scoring for the remainder, best-first
        let mut scored: Vec<(&SectionMeta, u32)> = self
            .index
            .sections()
            .iter()
            .filter(|m| !seen.contains(&m.id))
            .map(|m| (m, Self::keyword_score(request, m)))
            .filter(|(_, s)| *s > 0)
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        for (meta, _) in scored {
            if selected.len() >= max_sections + 2 {
                // +2 headroom for the two always-load sections
                break;
            }
            if seen.insert(meta.id.clone()) {
                selected.push(meta.id.clone());
            }
        }

        // 5. resolve dependencies (transitive, topological-ish: deps first)
        let mut ordered: Vec<String> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        fn visit(
            id: &str,
            index: &SkillIndex,
            visited: &mut HashSet<String>,
            ordered: &mut Vec<String>,
        ) {
            if !visited.insert(id.to_string()) {
                return;
            }
            if let Some(meta) = index.get(id) {
                for dep in &meta.requires {
                    visit(dep, index, visited, ordered);
                }
            }
            ordered.push(id.to_string());
        }
        for id in &selected {
            visit(id, self.index, &mut visited, &mut ordered);
        }

        // 6. load text
        let mut out = Vec::new();
        for id in ordered {
            let meta = match self.index.get(&id) {
                Some(m) => m,
                None => continue,
            };
            let text = if compact {
                self.index.read_compact(meta)
            } else {
                self.index.read_full(meta)
            }
            .unwrap_or_default();
            if text.is_empty() {
                continue;
            }
            let tokens = estimate_tokens(&text);
            out.push(RetrievedSection {
                id: id.clone(),
                text,
                tokens,
                from_explicit_ref: explicit.contains(&id),
            });
        }
        out
    }

    /// Resolve full text for one section by id (used for on-demand escalation
    /// from compact → full).
    pub fn read_full_section(&self, id: &str) -> Option<String> {
        let meta = self.index.get(id)?;
        self.index.read_full(meta).ok()
    }
}

/// Token estimate consistent with the rest of AETHER (chars/4).
pub fn estimate_tokens(s: &str) -> u32 {
    (s.chars().count() as u32) / 4
}

/// Why a section was selected (for debug view).
#[allow(dead_code)]
pub fn selection_reasons(request: &str, index: &SkillIndex) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for meta in index.sections() {
        if meta.always {
            out.insert(meta.id.clone(), "always-load".into());
        }
    }
    for r in SkillRetriever::explicit_refs(request) {
        out.insert(r, "explicit @ref".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_retriever() -> (SkillIndex, SkillRetriever<'static>) {
        // Leak the index for 'static lifetime in tests (test-only).
        let dir = SkillIndex::locate_bundled().expect("run from repo root");
        let idx: &'static SkillIndex =
            Box::leak(Box::new(SkillIndex::load(&dir).expect("index loads")));
        let r = SkillRetriever::new(idx);
        // Rebuild owned copies to avoid lifetime gymnastics in asserts
        let _ = r;
        let idx2: &'static SkillIndex =
            Box::leak(Box::new(SkillIndex::load(&dir).expect("index loads")));
        (SkillIndex::load(&dir).expect("index loads"), SkillRetriever::new(idx2))
    }

    #[test]
    fn bug_fix_loads_targeted_sections_only() {
        let (_idx, r) = test_retriever();
        let out = r.retrieve("Fix an authentication bug", 8, true);
        let ids: Vec<&str> = out.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"core"));
        assert!(ids.contains(&"testing"));
        assert!(ids.contains(&"security"));
        assert!(ids.contains(&"implementation"));
        assert!(!ids.contains(&"database"), "must not load database for auth bug");
        assert!(!ids.contains(&"adr"), "must not load adr for auth bug");
    }

    #[test]
    fn explicit_ref_loads_section() {
        let (_idx, r) = test_retriever();
        let out = r.retrieve("Do it per @software-engineering/testing please", 8, true);
        let ids: Vec<&str> = out.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"testing"));
        let t = out.iter().find(|s| s.id == "testing").unwrap();
        assert!(t.from_explicit_ref);
    }

    #[test]
    fn dependencies_resolve_before_dependents() {
        let (_idx, r) = test_retriever();
        let out = r.retrieve("Design a database schema", 8, true);
        let ids: Vec<&str> = out.iter().map(|s| s.id.as_str()).collect();
        let core_pos = ids.iter().position(|&x| x == "core").unwrap();
        let db_pos = ids.iter().position(|&x| x == "database").unwrap();
        assert!(core_pos < db_pos, "core dependency must precede database");
    }

    #[test]
    fn compact_costs_less_than_full() {
        let (_idx, r) = test_retriever();
        let compact: u32 = r.retrieve("Fix provider authentication", 8, true).iter().map(|s| s.tokens).sum();
        let full: u32 = r.retrieve("Fix provider authentication", 8, false).iter().map(|s| s.tokens).sum();
        assert!(compact < full, "compact must cost fewer tokens than full");
    }
}
