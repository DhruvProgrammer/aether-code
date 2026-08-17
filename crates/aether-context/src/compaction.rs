//! Compaction engine — the heart of context management.
//!
//! `CompactionEngine` is agent-aware. It can compact a single agent's
//! `ContextState` without touching any other agent's. Compaction is **never**
//! a blind truncation: it preserves the objective, plan, important decisions,
//! constraints, relevant files, unresolved errors and pinned segments, while
//! summarising or dropping older conversation / redundant tool outputs.

use crate::events::{
    CompactionEvent, ContextEventSink, NullSink, PreservationAction, PreservationRecord,
};
use crate::state::{ContextSegment, ContextSegmentKind, ContextState};
use crate::summarizer::{ExtractiveSummarizer, Summarizer};
use serde::{Deserialize, Serialize};

/// What triggered this compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTrigger {
    /// Proactive threshold reached (compact band).
    Threshold,
    /// Hard emergency threshold reached.
    Emergency,
    /// Explicit user / agent request.
    Manual,
    /// Before a snapshot (so the snapshot carries compacted context).
    PreSnapshot,
    /// Before a destructive operation.
    PreDanger,
    /// Before context handoff to another agent.
    Handoff,
}

impl CompactionTrigger {
    pub fn label(self) -> &'static str {
        match self {
            Self::Threshold => "threshold",
            Self::Emergency => "emergency",
            Self::Manual => "manual",
            Self::PreSnapshot => "pre_snapshot",
            Self::PreDanger => "pre_danger",
            Self::Handoff => "handoff",
        }
    }
}

/// Compaction strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStrategy {
    /// Summarise evictable segments, never drop the protected ones.
    Summarise,
    /// Drop the least-important evictable segments entirely.
    Drop,
    /// Mix: summarise the most recent evictable segment, drop older ones.
    Hybrid,
}

impl Default for CompactionStrategy {
    fn default() -> Self { Self::Hybrid }
}

fn is_protected(seg: &ContextSegment) -> bool {
    matches!(
        seg.kind,
        ContextSegmentKind::System
            | ContextSegmentKind::Objective
            | ContextSegmentKind::Plan
            | ContextSegmentKind::Constraint
            | ContextSegmentKind::UserRequirement
            | ContextSegmentKind::PendingWork
            | ContextSegmentKind::Error
            | ContextSegmentKind::RelevantFile
            | ContextSegmentKind::CodeSymbol
            | ContextSegmentKind::SkillBody
            | ContextSegmentKind::Decision
    )
}

fn is_evictable(seg: &ContextSegment) -> bool {
    matches!(
        seg.kind,
        ContextSegmentKind::Conversation
            | ContextSegmentKind::AgentMessage
            | ContextSegmentKind::ToolResult
            | ContextSegmentKind::MemoryRetrieval
            | ContextSegmentKind::CompletedWork
            | ContextSegmentKind::AgentFinding
            | ContextSegmentKind::UnresolvedQuestion
    )
}

/// The compactor itself. Cheap to clone (internally Arc'd).
#[derive(Clone)]
pub struct CompactionEngine {
    summarizer: std::sync::Arc<dyn Summarizer>,
    strategy: CompactionStrategy,
    sink: std::sync::Arc<dyn ContextEventSink>,
}

impl std::fmt::Debug for CompactionEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompactionEngine")
            .field("strategy", &self.strategy)
            .finish()
    }
}

impl Default for CompactionEngine {
    fn default() -> Self {
        Self {
            summarizer: std::sync::Arc::new(ExtractiveSummarizer::default()),
            strategy: CompactionStrategy::default(),
            sink: std::sync::Arc::new(NullSink),
        }
    }
}

impl CompactionEngine {
    pub fn new(summarizer: std::sync::Arc<dyn Summarizer>) -> Self {
        Self { summarizer, ..Self::default() }
    }

    pub fn with_strategy(mut self, s: CompactionStrategy) -> Self { self.strategy = s; self }
    pub fn with_sink(mut self, sink: std::sync::Arc<dyn ContextEventSink>) -> Self { self.sink = sink; self }

    /// Compact `state` in place and emit an event.
    pub fn compact(
        &self,
        state: &mut ContextState,
        session_id: &str,
        trigger: CompactionTrigger,
    ) -> CompactionEvent {
        let before = state.total_tokens();
        let mut preserved: u32 = 0;
        let mut summarised: u32 = 0;
        let mut dropped: u32 = 0;
        let mut preservation_map: std::collections::BTreeMap<ContextSegmentKind, (PreservationAction, u32)> =
            std::collections::BTreeMap::new();

        // Pass 1: drop oldest evictable kinds first.
        if matches!(self.strategy, CompactionStrategy::Drop | CompactionStrategy::Hybrid) {
            let kinds = [
                ContextSegmentKind::Conversation,
                ContextSegmentKind::AgentMessage,
                ContextSegmentKind::MemoryRetrieval,
                ContextSegmentKind::CompletedWork,
                ContextSegmentKind::UnresolvedQuestion,
                ContextSegmentKind::AgentFinding,
                ContextSegmentKind::ToolResult,
            ];
            for kind in kinds {
                let ids: Vec<String> = state
                    .segments
                    .iter()
                    .filter(|s| s.kind == kind && !s.meta.pinned && !is_protected(s))
                    .map(|s| s.meta.id.clone())
                    .collect();
                if ids.len() < 2 { continue; }
                let last = ids.last().cloned().unwrap_or_default();
                state.segments.retain(|s| {
                    if s.kind != kind { return true; }
                    if s.meta.pinned || is_protected(s) { return true; }
                    if s.meta.id == last { return true; }
                    state.usage.used = state.usage.used.saturating_sub(s.tokens);
                    false
                });
            }
        }

        // Pass 2: summarise long evictable segments. Synchronous summariser
        // call (the noop/extractive impls are CPU-only; the LLM summariser
        // would block here, but that's the intended trade-off — compactions
        // are gated on summariser latency anyway).
        if matches!(self.strategy, CompactionStrategy::Summarise | CompactionStrategy::Hybrid) {
            for seg in state.segments.iter_mut() {
                if seg.meta.pinned || is_protected(seg) { continue; }
                if !is_evictable(seg) { continue; }
                if seg.tokens < 200 { continue; } // nothing to summarise
                let hint = format!("kind={:?} title={}", seg.kind, seg.title);
                let original_len = seg.body.len();
                let new_body = futures::executor::block_on(self.summarizer.summarize(&seg.body, &hint));
                if new_body.len() < original_len {
                    let old_tokens = seg.tokens;
                    seg.body = new_body;
                    seg.tokens = estimate_tokens(&seg.body);
                    state.usage.used = state.usage.used
                        .saturating_sub(old_tokens)
                        .saturating_add(seg.tokens);
                }
            }
        }

        // Tally preservation.
        for seg in &state.segments {
            let action = if is_protected(seg) || seg.meta.pinned {
                preserved += 1;
                PreservationAction::Preserved
            } else if is_evictable(seg) {
                summarised += 1;
                PreservationAction::Summarised
            } else {
                PreservationAction::Preserved
            };
            preservation_map
                .entry(seg.kind)
                .and_modify(|(_, c)| *c += 1)
                .or_insert((action, 1));
        }
        let preservation: Vec<PreservationRecord> = preservation_map
            .into_iter()
            .map(|(kind, (action, segments))| PreservationRecord { kind, action, segments })
            .collect();

        let after = state.total_tokens();
        let event = CompactionEvent {
            session_id: session_id.to_string(),
            agent: state.owner.clone(),
            trigger,
            tokens_before: before,
            tokens_after: after,
            segments_kept: state.segments.len() as u32,
            segments_summarised: summarised,
            segments_dropped: dropped,
            preservation,
            timestamp: chrono::Utc::now(),
        };
        self.sink.on_compaction(&event);
        event
    }
}

/// Rough token estimator (chars/4). Used only inside the compactor; the
/// real count comes from the model's `usage` accounting.
pub fn estimate_tokens(body: &str) -> u32 {
    (body.len() as u32).div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::NullSink;
    use crate::summarizer::NoopSummarizer;

    fn build_state() -> ContextState {
        let mut s = ContextState::new("agent-a", "test-model", 1000);
        s.add(ContextSegment {
            kind: ContextSegmentKind::Objective,
            title: "objective".into(),
            body: "Build a thing".into(),
            tokens: 5,
            meta: crate::state::SegmentMeta::new("obj-1"),
        });
        for i in 0..20 {
            s.add(ContextSegment {
                kind: ContextSegmentKind::Conversation,
                title: format!("conv {i}"),
                body: "x".repeat(400),
                tokens: 100,
                meta: crate::state::SegmentMeta::new(format!("c-{i}")),
            });
        }
        s
    }

    #[test]
    fn compaction_preserves_objective() {
        let engine = CompactionEngine::default();
        let mut s = build_state();
        let ev = engine.compact(&mut s, "sess", CompactionTrigger::Threshold);
        assert!(s.segments.iter().any(|seg| seg.kind == ContextSegmentKind::Objective));
        assert!(ev.tokens_after < ev.tokens_before);
    }

    #[test]
    fn compaction_emits_event() {
        let engine = CompactionEngine::default();
        let mut s = build_state();
        let ev = engine.compact(&mut s, "sess-1", CompactionTrigger::Manual);
        assert_eq!(ev.session_id, "sess-1");
        assert_eq!(ev.agent, "agent-a");
        assert_eq!(ev.trigger, CompactionTrigger::Manual);
    }

    #[test]
    fn pinned_segments_are_kept() {
        let engine = CompactionEngine::new(std::sync::Arc::new(NoopSummarizer))
            .with_strategy(CompactionStrategy::Drop);
        let mut s = ContextState::new("a", "m", 1000);
        s.add(ContextSegment {
            kind: ContextSegmentKind::Conversation,
            title: "pinned".into(),
            body: "x".repeat(800),
            tokens: 200,
            meta: crate::state::SegmentMeta::new("p-1").pinned(),
        });
        for i in 0..10 {
            s.add(ContextSegment {
                kind: ContextSegmentKind::Conversation,
                title: format!("c {i}"),
                body: "y".repeat(800),
                tokens: 200,
                meta: crate::state::SegmentMeta::new(format!("c-{i}")),
            });
        }
        engine.compact(&mut s, "s", CompactionTrigger::Threshold);
        assert!(s.segments.iter().any(|seg| seg.meta.id == "p-1"));
    }
}
