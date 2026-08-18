//! `aether-context` — automatic context management + compaction engine.
//!
//! Production-grade, agent-aware context subsystem for AETHER. Replaces the
//! ad-hoc `compact_messages` heuristic inside the Executor with a first-class
//! `ContextManager` that tracks every context contributor (system, instructions,
//! memory retrieval, skill bodies, conversation history, tool calls and tool
//! outputs, file/code references, inter-agent messages) and triggers structured
//! compaction before the model's real window is breached.
//!
//! # Design
//!
//! * **Agent-aware** — each LLM (controller, executor, reviewer) and each
//!   specialist agent owns a [`ContextManager`]. The agent can compact its own
//!   context without disturbing the controller's.
//! * **Structured** — context is held as a [`ContextState`] of named segments,
//!   not a flat `Vec<Message>`. Compaction works by reorganising / summarising
//!   segments, never by blindly truncating `Message::content`.
//! * **Thresholded** — three configurable thresholds (warn / compact /
//!   emergency) expressed as a fraction of the model's actual `context_window`.
//! * **Pluggable** — [`Summarizer`] is a trait so the runtime can swap local
//!   extractive summarisation, LLM-backed summarisation, or hybrid.
//! * **Observable** — every compaction emits a [`CompactionEvent`] on an
//!   optional event sink for the UI / TUI / desktop app.

pub mod state;
pub mod manager;
pub mod compaction;
pub mod events;
pub mod summarizer;
pub mod thresholds;
pub mod workspace;

pub use state::{
    ContextSegment, ContextSegmentKind, ContextState, SegmentMeta, TokenUsage,
};
pub use manager::{ContextManager, ContextManagerConfig};
pub use compaction::{CompactionEngine, CompactionStrategy, CompactionTrigger};
pub use events::{CompactionEvent, ContextEventSink, NullSink};
pub use summarizer::{ExtractiveSummarizer, NoopSummarizer, Summarizer};
pub use thresholds::{ContextThresholds, ThresholdAction};
pub use workspace::{AgentContextStatus, ContextWorkspace, SharedSegment};
