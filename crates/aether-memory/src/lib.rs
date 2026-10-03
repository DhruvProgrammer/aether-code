//! `aether-memory` — hierarchical context memory & relevant retrieval engine.
//!
//! The complete history stays stored (Layer 5, in `aether-sessions`); this
//! crate decides which *meaningful units* are worth keeping as searchable
//! memory (Layers 2–4) and which subset enters the small active working
//! context (Layer 0) for the current request.
//!
//! Pure logic: no provider calls, no UI. Retrieval is heuristic
//! (lexical BM25 via SQLite FTS5 + recency/importance/task/file signals +
//! penalties) so it works offline and stays testable. LLM2 may later own
//! semantic extraction; the rule-based extractor here is the always-on
//! fast path, especially for user corrections which must never be lost.
//!
//! Arch docs: `reference_architecture/MEMORY_*.md`.

pub mod engine;
pub mod extract;
pub mod retrieve;
pub mod store;
pub mod types;

pub use engine::MemoryEngine;
pub use extract::{ExtractContext, MessageView};
pub use retrieve::{QueryProfile, Retrieval, RetrievalOptions, Weights};
pub use store::{MemoryError, MemoryStore, SqliteMemoryStore};
pub use types::{MemoryRecord, MemoryStatus, MemoryType, Relation};

/// Rough token estimate (chars/4), consistent with the rest of the tree.
pub fn estimate_tokens(s: &str) -> u32 {
    (s.chars().count() as u32).saturating_add(3) / 4
}
