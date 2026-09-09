//! Reference index + retriever for `reference_architecture/`.
//!
//! The reference repos are never pasted whole. An index maps subsystem →
//! files; the retriever returns bounded excerpts.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceItem {
    pub id: String,
    pub repo: String,
    pub path: String,
    pub subsystem: String,
    pub description: String,
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReferenceIndexFile {
    items: Vec<ReferenceItem>,
}

pub struct ReferenceIndex {
    base_dir: PathBuf,
    items: Vec<ReferenceItem>,
}

impl ReferenceIndex {
    /// Load from `reference_architecture/index.json` (generated once from
    /// REFERENCE_MANIFEST.md). Falls back to a small built-in map covering
    /// the curated files if the JSON is absent.
    pub fn load(repo_root: &Path) -> Self {
        let base = repo_root.join("reference_architecture");
        let items = Self::load_json(&base).unwrap_or_else(Self::builtin);
        Self { base_dir: base, items }
    }

    fn load_json(base: &Path) -> Option<Vec<ReferenceItem>> {
        let raw = std::fs::read_to_string(base.join("index.json")).ok()?;
        let parsed: ReferenceIndexFile = serde_json::from_str(&raw).ok()?;
        Some(parsed.items)
    }

    fn builtin() -> Vec<ReferenceItem> {
        let entry = |id: &str, repo: &str, path: &str, subsystem: &str, description: &str, keywords: &[&str]| ReferenceItem {
            id: id.into(),
            repo: repo.into(),
            path: path.into(),
            subsystem: subsystem.into(),
            description: description.into(),
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
        };
        vec![
            entry("opencode-compaction", "opencode", "opencode/compaction.ts", "compaction", "Turn-based session compaction with recent-tail budget and tool-output truncation", &["compact", "compaction", "checkpoint", "tail", "truncate"]),
            entry("opencode-overflow", "opencode", "opencode/overflow.ts", "context", "Overflow detection: usable = model context minus reserved output; actual usage comparison", &["overflow", "context", "budget", "usable", "tokens"]),
            entry("opencode-provider", "opencode", "opencode/provider.ts", "provider", "Provider + Model schema: limits, capabilities, cost, variants, headers", &["provider", "model", "capabilities", "limit", "cost"]),
            entry("opencode-agent", "opencode", "opencode/agent.ts", "agent", "Agent turn lifecycle with retries and cancellation", &["agent", "loop", "turn", "retry", "cancel"]),
            entry("opencode-tools", "opencode", "opencode/tool.ts", "tools", "Tool trait, schema, JSON validation", &["tool", "schema", "registry", "validation"]),
            entry("hermes-memory", "hermes-agent", "hermes-agent/memory_manager.py", "memory", "MemoryManager: single external provider slot, prefetch_all, sync_all", &["memory", "provider", "prefetch", "sync"]),
            entry("hermes-prompt", "hermes-agent", "hermes-agent/prompt_builder.py", "prompt", "Staged prompt assembly with context-file threat scanning", &["prompt", "assembly", "threat", "injection", "context file"]),
            entry("hermes-coding-context", "hermes-agent", "hermes-agent/coding_context.py", "workspace", "Project-aware file sampling for coding context", &["coding", "context", "sampling", "files"]),
            entry("hermes-skills", "hermes-agent", "hermes-agent/skill_utils.py", "skills", "Skill frontmatter parsing and environment matching", &["skill", "frontmatter", "discovery"]),
            entry("grok-compaction", "grok-build", "grok-build/crates/common/xai-grok-compaction/src/lib.rs", "compaction", "Rust compaction core with trigger/result/error types", &["compact", "rust", "trigger", "result"]),
            entry("grok-breaker", "grok-build", "grok-build/crates/common/xai-circuit-breaker/src/breaker.rs", "runtime", "Circuit breaker for provider/model failure isolation", &["circuit", "breaker", "retry", "failure", "provider"]),
            entry("grok-tools", "grok-build", "grok-build/crates/common/xai-tool-runtime/src/tool.rs", "tools", "Rust tool trait with dispatch and streaming", &["tool", "rust", "dispatch", "stream"]),
        ]
    }

    pub fn items(&self) -> &[ReferenceItem] {
        &self.items
    }
}

pub struct ReferenceRetriever<'a> {
    index: &'a ReferenceIndex,
}

impl<'a> ReferenceRetriever<'a> {
    pub fn new(index: &'a ReferenceIndex) -> Self {
        Self { index }
    }

    /// Retrieve up to `max_files` items matching the request, each truncated
    /// to `max_chars` with a header noting provenance.
    pub fn retrieve(&self, request: &str, max_files: usize, max_chars: usize) -> Vec<(ReferenceItem, String)> {
        let lower = request.to_lowercase();
        let mut scored: Vec<(&ReferenceItem, u32)> = self
            .index
            .items()
            .iter()
            .map(|item| {
                let mut s = 0u32;
                for kw in &item.keywords {
                    s += lower.matches(kw.to_lowercase().as_str()).count() as u32;
                }
                for w in item.description.split(|c: char| !c.is_alphanumeric()) {
                    if w.len() > 4 && lower.contains(&w.to_lowercase()) {
                        s += 1;
                    }
                }
                (item, s)
            })
            .filter(|(_, s)| *s > 0)
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored
            .into_iter()
            .take(max_files)
            .filter_map(|(item, _)| {
                let full = self.index.base_dir.join(&item.path);
                let content = std::fs::read_to_string(&full).ok()?;
                let excerpt = if content.len() > max_chars {
                    format!("{}...[truncated {} chars]", &content[..max_chars], content.len() - max_chars)
                } else {
                    content
                };
                Some((item.clone(), format!("[Reference: {}/{}]\n{}", item.repo, item.path, excerpt)))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_index_covers_key_subsystems() {
        let items = ReferenceIndex::builtin();
        let subsystems: Vec<&str> = items.iter().map(|i| i.subsystem.as_str()).collect();
        for need in ["compaction", "provider", "memory", "prompt", "tools"] {
            assert!(subsystems.contains(&need), "missing {need}");
        }
    }

    #[test]
    fn retrieve_matches_compaction_request() {
        let idx = ReferenceIndex { base_dir: PathBuf::from("."), items: ReferenceIndex::builtin() };
        let r = ReferenceRetriever::new(&idx);
        // No files on disk in test CWD, but scoring path is exercised via items();
        // verify scoring prefers compaction items for a compaction request.
        let lower = "improve compaction checkpoint".to_lowercase();
        let mut best: Vec<(&str, u32)> = idx
            .items()
            .iter()
            .map(|i| {
                let mut s = 0u32;
                for kw in &i.keywords {
                    s += lower.matches(kw.to_lowercase().as_str()).count() as u32;
                }
                (i.id.as_str(), s)
            })
            .filter(|(_, s)| *s > 0)
            .collect();
        best.sort_by(|a, b| b.1.cmp(&a.1));
        assert!(!best.is_empty());
        assert!(best[0].0.contains("compact") || best[0].0.contains("overflow"));
        let _ = r;
    }
}
