//! Structured context state.
//!
//! A [`ContextState`] holds the live LLM context as a list of named segments.
//! Each segment has a [`ContextSegmentKind`], token accounting and a priority
//! tag so the compactor knows what to preserve.

use serde::{Deserialize, Serialize};

/// Kinds of context segment. Ordered loosely by importance for the default
/// compaction priority (earlier kinds survive compactions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSegmentKind {
    /// The system prompt / persona. Never compacted.
    System,
    /// The user's original objective / current task statement. Survives every
    /// compaction verbatim.
    Objective,
    /// Active plan (ordered steps). Survives every compaction verbatim.
    Plan,
    /// Completed-work ledger. Summarised when too large.
    CompletedWork,
    /// Pending-work ledger. Survives until empty.
    PendingWork,
    /// Important decisions the model has made. Each entry preserved.
    Decision,
    /// Hard constraints (read-only, no-deploy, etc.) carried into every call.
    Constraint,
    /// Files / code symbols currently being modified. Preserved verbatim while
    /// in scope.
    RelevantFile,
    /// Code symbols the model is actively reasoning about.
    CodeSymbol,
    /// Errors encountered and still relevant.
    Error,
    /// Findings from specialist sub-agents. Marked-important entries survive.
    AgentFinding,
    /// Tool results. Large outputs may be replaced with structured summaries.
    ToolResult,
    /// User requirements / clarifications.
    UserRequirement,
    /// Open questions.
    UnresolvedQuestion,
    /// Skills that have been loaded into context for this agent.
    SkillBody,
    /// Memory retrieval payloads (RAG). Summarisable.
    MemoryRetrieval,
    /// Older conversation / tool history. The prime compaction target.
    Conversation,
    /// Inter-agent messages (controller ↔ specialist).
    AgentMessage,
}

impl ContextSegmentKind {
    /// Default preservation priority. Lower = more important.
    /// Used by the compactor to pick which segments to keep verbatim vs
    /// summarise vs drop.
    pub fn default_priority(self) -> u8 {
        match self {
            Self::System => 0,
            Self::Objective => 1,
            Self::Constraint => 2,
            Self::Plan => 3,
            Self::UserRequirement => 4,
            Self::PendingWork => 5,
            Self::Error => 6,
            Self::RelevantFile => 7,
            Self::CodeSymbol => 8,
            Self::AgentFinding => 9,
            Self::Decision => 10,
            Self::SkillBody => 11,
            Self::ToolResult => 12,
            Self::MemoryRetrieval => 13,
            Self::CompletedWork => 14,
            Self::AgentMessage => 15,
            Self::UnresolvedQuestion => 16,
            Self::Conversation => 17,
        }
    }
}

/// Per-segment metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentMeta {
    /// Optional stable id (e.g. `tool:<call_id>`, `file:<path>`, `skill:<id>`).
    pub id: String,
    /// Whether the segment is pinned (must survive compaction).
    pub pinned: bool,
    /// Optional importance score in `[0.0, 1.0]`. Used by the compactor to
    /// rank peer segments of the same kind.
    pub importance: f32,
}

impl SegmentMeta {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), pinned: false, importance: 0.5 }
    }
    pub fn pinned(mut self) -> Self { self.pinned = true; self }
    pub fn with_importance(mut self, imp: f32) -> Self { self.importance = imp.clamp(0.0, 1.0); self }
}

/// A single segment of context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSegment {
    pub kind: ContextSegmentKind,
    pub title: String,
    pub body: String,
    pub tokens: u32,
    pub meta: SegmentMeta,
}

/// Token accounting for a context.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub used: u32,
    pub limit: u32,
}

impl TokenUsage {
    pub fn pct(&self) -> f32 {
        if self.limit == 0 { return 0.0; }
        self.used as f32 / self.limit as f32
    }
}

/// The full state of one agent's context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextState {
    pub owner: String,
    pub segments: Vec<ContextSegment>,
    pub usage: TokenUsage,
    pub model: String,
}

impl ContextState {
    pub fn new(owner: impl Into<String>, model: impl Into<String>, limit: u32) -> Self {
        Self {
            owner: owner.into(),
            segments: Vec::new(),
            usage: TokenUsage { used: 0, limit },
            model: model.into(),
        }
    }

    pub fn add(&mut self, seg: ContextSegment) {
        self.usage.used = self.usage.used.saturating_add(seg.tokens);
        self.segments.push(seg);
    }

    pub fn total_tokens(&self) -> u32 {
        self.usage.used
    }

    /// Find segments by kind.
    pub fn find_by_kind(&self, kind: ContextSegmentKind) -> impl Iterator<Item = &ContextSegment> {
        self.segments.iter().filter(move |s| s.kind == kind)
    }
}
