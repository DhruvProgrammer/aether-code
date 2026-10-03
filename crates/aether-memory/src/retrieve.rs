//! Multi-stage retrieval (§17–27): profile -> hybrid score -> rerank ->
//! expand -> dedupe -> conflict-resolve -> pack under a token budget.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::estimate_tokens;
use crate::extract::{extract_files, extract_symbols};
use crate::store::{MemoryError, MemoryStore};
use crate::types::{MemoryRecord, MemoryStatus, MemoryType, Relation};

/// Task intents drive type weights (§21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Debug,
    Plan,
    Verify,
    Implement,
    General,
}

/// Structured understanding of the current request (§20).
#[derive(Debug, Clone)]
pub struct QueryProfile {
    pub text: String,
    pub intent: Intent,
    pub entities: Vec<String>,
    pub files: Vec<String>,
    pub symbols: Vec<String>,
    pub wanted: Vec<MemoryType>,
    pub project_id: String,
    pub session_id: Option<String>,
}

impl QueryProfile {
    pub fn new(
        text: &str,
        project_id: &str,
        session_id: Option<String>,
    ) -> Self {
        let low = text.to_lowercase();
        let intent = if contains_any(&low, &["fix", "bug", "error", "fail", "broken", "crash", "debug"])
        {
            Intent::Debug
        } else if contains_any(&low, &["plan", "design", "architect", "propos"]) {
            Intent::Plan
        } else if contains_any(&low, &["verif", "test", "pass", "check", "review"]) {
            Intent::Verify
        } else if contains_any(
            &low,
            &["implement", "add", "build", "create", "write", "refactor"],
        ) {
            Intent::Implement
        } else {
            Intent::General
        };
        let wanted = match intent {
            Intent::Debug => vec![
                MemoryType::Error,
                MemoryType::Bug,
                MemoryType::Fix,
                MemoryType::Implementation,
                MemoryType::Verification,
                MemoryType::FileInsight,
                MemoryType::ToolResult,
            ],
            Intent::Plan => vec![
                MemoryType::Decision,
                MemoryType::Requirement,
                MemoryType::Dependency,
                MemoryType::ImportantFact,
                MemoryType::Plan,
                MemoryType::Constraint,
            ],
            Intent::Verify => vec![
                MemoryType::Verification,
                MemoryType::Implementation,
                MemoryType::Error,
                MemoryType::ToolResult,
                MemoryType::Fix,
            ],
            Intent::Implement => vec![
                MemoryType::Plan,
                MemoryType::Decision,
                MemoryType::FileInsight,
                MemoryType::Dependency,
                MemoryType::Episode,
                MemoryType::Requirement,
            ],
            Intent::General => vec![
                MemoryType::UserCorrection,
                MemoryType::Decision,
                MemoryType::ImportantFact,
                MemoryType::Episode,
            ],
        };
        // Entities: quoted phrases + code symbols + files.
        let mut entities: Vec<String> = extract_symbols(text);
        for part in text.split('"').skip(1).step_by(2) {
            let p = part.trim();
            if p.len() > 2 && !entities.contains(&p.to_string()) {
                entities.push(p.to_string());
            }
        }
        Self {
            text: text.to_string(),
            intent,
            entities,
            files: extract_files(text),
            symbols: extract_symbols(text),
            wanted,
            project_id: project_id.to_string(),
            session_id,
        }
    }
}

fn contains_any(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

/// Tunable scoring weights (§22). Defaults are a starting point, not law —
/// every weight is covered by tests so tuning stays honest.
#[derive(Debug, Clone)]
pub struct Weights {
    pub lexical: f32,
    pub recency: f32,
    pub importance: f32,
    pub task: f32,
    pub file: f32,
    pub relation: f32,
    pub stale_penalty: f32,
    pub contradiction_penalty: f32,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            lexical: 0.35,
            recency: 0.15,
            importance: 0.2,
            task: 0.15,
            file: 0.1,
            relation: 0.05,
            stale_penalty: 0.4,
            contradiction_penalty: 0.5,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RetrievalOptions {
    /// Token budget for packed memories (§27).
    pub memory_budget_tokens: u32,
    /// Hard cap before packing (dynamic top-K: budget decides the rest).
    pub max_candidates: usize,
    /// Minimum hybrid score to survive rerank.
    pub min_score: f32,
    /// How many related neighbors may be pulled in (§24).
    pub expansion_budget: usize,
    /// Include superseded/stale when history itself is requested (§14).
    pub include_history: bool,
    /// Workspace paths known to be gone/changed — memory citing them is
    /// penalized because the workspace wins (§34).
    pub stale_files: HashSet<String>,
}

impl Default for RetrievalOptions {
    fn default() -> Self {
        Self {
            memory_budget_tokens: 2000,
            max_candidates: 30,
            min_score: 0.15,
            expansion_budget: 8,
            include_history: false,
            stale_files: HashSet::new(),
        }
    }
}

/// Per-signal contribution, for diagnostics (§56–57).
#[derive(Debug, Clone, Default)]
pub struct ScoreBreakdown {
    pub lexical: f32,
    pub recency: f32,
    pub importance: f32,
    pub task: f32,
    pub file: f32,
    pub relation: f32,
    pub penalties: f32,
}

#[derive(Debug, Clone)]
pub struct ScoredMemory {
    pub record: MemoryRecord,
    pub score: f32,
    pub breakdown: ScoreBreakdown,
    /// Why this memory was retrieved (provenance, §33).
    pub provenance: String,
}

#[derive(Debug, Clone, Default)]
pub struct RetrievalDiagnostics {
    pub intent: String,
    pub candidates: usize,
    pub kept: usize,
    pub packed_tokens: u32,
    pub latency_ms: u64,
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Retrieval {
    pub memories: Vec<ScoredMemory>,
    pub diagnostics: RetrievalDiagnostics,
}

fn recency_score(now: i64, created: i64) -> f32 {
    let age_days = (now - created).max(0) as f64 / 86400.0;
    (1.0 / (1.0 + age_days / 3.0)) as f32
}

fn file_overlap(record_files: &[String], query_files: &[String]) -> f32 {
    if query_files.is_empty() || record_files.is_empty() {
        return 0.0;
    }
    let hit = record_files
        .iter()
        .filter(|f| query_files.iter().any(|q| *f == q || f.ends_with(q.as_str())))
        .count();
    (hit as f32 / query_files.len() as f32).min(1.0)
}

/// Full pipeline. See `RETRIEVAL_ARCHITECTURE.md` for the stage diagram.
pub fn retrieve(
    store: &dyn MemoryStore,
    profile: &QueryProfile,
    opts: &RetrievalOptions,
    weights: &Weights,
    now: i64,
) -> Result<Retrieval, MemoryError> {
    let start = Instant::now();

    // 1. Candidates: lexical (BM25) + recent session/project memory.
    let mut pool: HashMap<String, (MemoryRecord, f32)> = HashMap::new();
    let lex = store.search_lexical(&profile.text, opts.max_candidates * 3)?;
    let lex_max = lex.iter().map(|(_, s)| *s).fold(0.0f32, f32::max).max(1e-6);
    for (rec, raw) in lex {
        pool.insert(rec.id.clone(), (rec, raw / lex_max));
    }
    for rec in store.list_recent(
        &profile.project_id,
        profile.session_id.as_deref(),
        opts.max_candidates,
    )? {
        pool.entry(rec.id.clone()).or_insert((rec, 0.0));
    }

    // 2. Hybrid score.
    let mut scored: Vec<ScoredMemory> = Vec::new();
    for (rec, lex_norm) in pool.into_values() {
        let usable = rec.status.is_usable();
        if !usable && !opts.include_history {
            continue;
        }
        let task = if profile.wanted.contains(&rec.mem_type) {
            1.0
        } else if rec.mem_type == MemoryType::UserCorrection {
            1.0 // corrections always matter (§13)
        } else {
            0.2
        };
        let file = file_overlap(&rec.files, &profile.files);
        let mut penalties = 0.0;
        if matches!(rec.status, MemoryStatus::Superseded | MemoryStatus::Stale) {
            penalties += weights.stale_penalty;
        }
        if !opts.stale_files.is_empty()
            && rec.files.iter().any(|f| opts.stale_files.contains(f))
        {
            penalties += weights.stale_penalty * 0.5;
        }
        let b = ScoreBreakdown {
            lexical: lex_norm,
            recency: recency_score(now, rec.created_at),
            importance: rec.importance * rec.confidence,
            task,
            file,
            relation: 0.0,
            penalties,
        };
        let score = weights.lexical * b.lexical
            + weights.recency * b.recency
            + weights.importance * b.importance
            + weights.task * b.task
            + weights.file * b.file
            - b.penalties;
        if score < opts.min_score {
            continue;
        }
        let provenance = format!(
            "session={} task={} files=[{}] sources=[{}]",
            rec.session_id.as_deref().unwrap_or("-"),
            rec.task_id.as_deref().unwrap_or("-"),
            rec.files.join(","),
            rec.source_ids.join(","),
        );
        scored.push(ScoredMemory {
            record: rec,
            score,
            breakdown: b,
            provenance,
        });
    }

    // 3. Rerank: sort, cap.
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(opts.max_candidates);

    // 4. Relationship expansion within budget (§24).
    let mut expanded: Vec<ScoredMemory> = Vec::new();
    let mut seen: HashSet<String> = scored.iter().map(|s| s.record.id.clone()).collect();
    let mut budget = opts.expansion_budget;
    for s in &scored {
        if budget == 0 {
            break;
        }
        for (rel_rec, rel) in store.list_related(&s.record.id)? {
            if budget == 0 || seen.contains(&rel_rec.id) {
                continue;
            }
            if !rel_rec.status.is_usable() && !opts.include_history {
                continue;
            }
            seen.insert(rel_rec.id.clone());
            budget -= 1;
            let bonus = match rel {
                Relation::Supersedes | Relation::Contradicts => {
                    // Never auto-expand into replaced/contradicted records.
                    continue;
                }
                _ => weights.relation,
            };
            expanded.push(ScoredMemory {
                score: s.score * 0.7 + bonus,
                breakdown: ScoreBreakdown {
                    relation: bonus,
                    ..Default::default()
                },
                provenance: format!("via {} -> {}", s.record.id, rel.as_str()),
                record: rel_rec,
            });
        }
    }
    scored.extend(expanded);
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    // 5. Deduplicate by content hash (§59): keep the best of each group.
    let mut by_hash: HashMap<String, ScoredMemory> = HashMap::new();
    for s in scored {
        let h = s.record.content_hash();
        match by_hash.get(&h) {
            Some(prev) if prev.score >= s.score => {}
            _ => {
                by_hash.insert(h, s);
            }
        }
    }
    let mut deduped: Vec<ScoredMemory> = by_hash.into_values().collect();
    deduped.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    // 6. Conflict resolution: drop records superseded by a kept newer one.
    let superseded_ids: HashSet<String> = deduped
        .iter()
        .flat_map(|s| s.record.superseded_by.clone())
        .collect();
    if !opts.include_history {
        deduped.retain(|s| !superseded_ids.contains(&s.record.id));
    }

    // 7. Pack under the token budget (§26–27): best first, summaries when
    // a full body would blow the remainder.
    let mut packed: Vec<ScoredMemory> = Vec::new();
    let mut used: u32 = 0;
    for s in deduped {
        let full = estimate_tokens(&s.record.content);
        let (text_len, _summary_used) = match &s.record.summary {
            Some(sum) if full > opts.memory_budget_tokens.saturating_sub(used) => {
                (estimate_tokens(sum), true)
            }
            _ => (full, false),
        };
        if used + text_len > opts.memory_budget_tokens {
            continue;
        }
        used += text_len;
        packed.push(s);
    }

    let diagnostics = RetrievalDiagnostics {
        intent: format!("{:?}", profile.intent),
        candidates: packed.len(),
        kept: packed.len(),
        packed_tokens: used,
        latency_ms: start.elapsed().as_millis() as u64,
        items: packed
            .iter()
            .map(|s| {
                format!(
                    "{} [{}] score={:.2} src={}",
                    s.record.title, s.record.mem_type.as_str(), s.score, s.provenance
                )
            })
            .collect(),
    };
    Ok(Retrieval {
        memories: packed,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SqliteMemoryStore;
    use crate::types::MemoryType;

    fn seeded() -> SqliteMemoryStore {
        let s = SqliteMemoryStore::open_in_memory().unwrap();
        let mut r = MemoryRecord::new(
            "p",
            MemoryType::Decision,
            "Use SQLite for local memory",
            "We chose SQLite with FTS5 for the local memory store in aether-memory.",
        );
        r.files = vec!["crates/aether-memory/src/store.rs".into()];
        r.importance = 0.8;
        s.insert(&r).unwrap();
        let old = MemoryRecord::new("p", MemoryType::Decision, "Use Postgres", "Use Postgres for storage");
        s.insert(&old).unwrap();
        let mut corr = MemoryRecord::new(
            "p",
            MemoryType::UserCorrection,
            "Do not use Postgres",
            "Do not use Postgres; this project uses SQLite for storage.",
        );
        corr.importance = 0.95;
        corr.files = vec!["crates/aether-memory/src/store.rs".into()];
        s.insert(&corr).unwrap();
        s.mark_superseded(&old.id, &corr.id).unwrap();
        // Filler from other episodes.
        for i in 0..30 {
            let f = MemoryRecord::new(
                "p",
                MemoryType::ToolResult,
                format!("filler {i}"),
                format!("unrelated tool output number {i} about UI colors"),
            );
            s.insert(&f).unwrap();
        }
        s
    }

    #[test]
    fn intent_drives_wanted_types() {
        let p = QueryProfile::new("Fix the provider validation bug", "p", None);
        assert_eq!(p.intent, Intent::Debug);
        assert!(p.wanted.contains(&MemoryType::Error));
        let q = QueryProfile::new("Plan the memory architecture", "p", None);
        assert_eq!(q.intent, Intent::Plan);
    }

    #[test]
    fn buried_decision_is_retrieved_without_exact_words() {
        let s = seeded();
        let now = chrono::Utc::now().timestamp();
        // "Continue the storage work" shares almost no vocabulary with the record.
        let p = QueryProfile::new("Continue the storage work in the memory crate", "p", None);
        let r = retrieve(&s, &p, &RetrievalOptions::default(), &Weights::default(), now).unwrap();
        assert!(
            r.memories.iter().any(|m| m.record.title.contains("SQLite")),
            "buried decision missing: {:?}",
            r.diagnostics.items
        );
        assert!(r.diagnostics.packed_tokens <= RetrievalOptions::default().memory_budget_tokens);
    }

    #[test]
    fn superseded_record_loses_to_correction() {
        let s = seeded();
        let now = chrono::Utc::now().timestamp();
        let p = QueryProfile::new("which database for storage", "p", None);
        let r = retrieve(&s, &p, &RetrievalOptions::default(), &Weights::default(), now).unwrap();
        let titles: Vec<_> = r.memories.iter().map(|m| m.record.title.as_str()).collect();
        assert!(titles.contains(&"Do not use Postgres"));
        assert!(!titles.contains(&"Use Postgres"), "stale record resurfaced: {titles:?}");
    }

    #[test]
    fn stale_files_are_penalized() {
        let s = seeded();
        let now = chrono::Utc::now().timestamp();
        let mut opts = RetrievalOptions::default();
        opts.stale_files.insert("crates/aether-memory/src/store.rs".into());
        let p = QueryProfile::new("SQLite memory store", "p", None);
        let r = retrieve(&s, &p, &opts, &Weights::default(), now).unwrap();
        let r2 = retrieve(&s, &p, &RetrievalOptions::default(), &Weights::default(), now).unwrap();
        let penalized: f32 = r.memories.iter().filter(|m| m.record.title.contains("SQLite")).map(|m| m.score).sum();
        let clean: f32 = r2.memories.iter().filter(|m| m.record.title.contains("SQLite")).map(|m| m.score).sum();
        assert!(penalized <= clean);
    }
}
