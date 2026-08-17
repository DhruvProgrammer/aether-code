//! `ContextManager` — per-agent (or global) facade over `ContextState` +
//! `CompactionEngine`. Re-exported by the crate root.

use crate::compaction::{CompactionEngine, CompactionTrigger};
use crate::events::{CompactionEvent, ContextEventSink, NullSink};
use crate::state::{ContextSegment, ContextState, TokenUsage};
use crate::thresholds::{ContextThresholds, ThresholdAction};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextManagerConfig {
    pub owner: String,
    pub model: String,
    pub context_window: u32,
    pub thresholds: ContextThresholds,
}

impl ContextManagerConfig {
    pub fn new(owner: impl Into<String>, model: impl Into<String>, context_window: u32) -> Self {
        Self {
            owner: owner.into(),
            model: model.into(),
            context_window,
            thresholds: ContextThresholds::default(),
        }
    }
}

pub struct ContextManager {
    cfg: ContextManagerConfig,
    state: Mutex<ContextState>,
    engine: CompactionEngine,
    sink: Arc<dyn ContextEventSink>,
    last_event: Mutex<Option<CompactionEvent>>,
}

impl std::fmt::Debug for ContextManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextManager")
            .field("owner", &self.cfg.owner)
            .field("model", &self.cfg.model)
            .field("thresholds", &self.cfg.thresholds)
            .finish()
    }
}

impl ContextManager {
    pub fn new(cfg: ContextManagerConfig) -> Self {
        let state = ContextState::new(cfg.owner.clone(), cfg.model.clone(), cfg.context_window);
        Self {
            cfg,
            state: Mutex::new(state),
            engine: CompactionEngine::default(),
            sink: Arc::new(NullSink),
            last_event: Mutex::new(None),
        }
    }

    pub fn with_engine(mut self, engine: CompactionEngine) -> Self { self.engine = engine; self }
    pub fn with_sink(mut self, sink: Arc<dyn ContextEventSink>) -> Self { self.sink = sink; self }

    pub fn config(&self) -> &ContextManagerConfig { &self.cfg }
    pub fn thresholds(&self) -> &ContextThresholds { &self.cfg.thresholds }
    pub fn model(&self) -> &str { &self.cfg.model }
    pub fn owner(&self) -> &str { &self.cfg.owner }

    /// Snapshot of the current state.
    pub fn snapshot(&self) -> ContextState { self.state.lock().clone() }

    /// Current usage fraction (0..=1+).
    pub fn usage_pct(&self) -> f32 { self.state.lock().usage.pct() }

    /// Add a segment. Caller supplies an estimated token count; the manager
    /// updates its running total. Compaction is NOT triggered automatically —
    /// callers should call [`Self::check_and_compact`] after significant
    /// additions.
    pub fn push(&self, seg: ContextSegment) {
        let mut s = self.state.lock();
        s.usage.used = s.usage.used.saturating_add(seg.tokens);
        s.segments.push(seg);
    }

    /// Classify current pressure.
    pub fn pressure(&self) -> ThresholdAction {
        let pct = self.usage_pct();
        self.cfg.thresholds.classify(pct)
    }

    /// Run a compaction if pressure warrants it.
    /// Returns `Some(event)` when a compaction ran, `None` otherwise.
    pub fn check_and_compact(&self, session_id: &str) -> Option<CompactionEvent> {
        let action = self.pressure();
        let trigger = match action {
            ThresholdAction::None | ThresholdAction::Warn => return None,
            ThresholdAction::Compact => CompactionTrigger::Threshold,
            ThresholdAction::Emergency => CompactionTrigger::Emergency,
        };
        let mut s = self.state.lock();
        let event = self.engine.compact(&mut s, session_id, trigger);
        *self.last_event.lock() = Some(event.clone());
        Some(event)
    }

    /// Force a compaction (manual / pre-snapshot / pre-danger).
    pub fn force_compact(&self, session_id: &str, trigger: CompactionTrigger) -> CompactionEvent {
        let mut s = self.state.lock();
        let event = self.engine.compact(&mut s, session_id, trigger);
        *self.last_event.lock() = Some(event.clone());
        event
    }

    /// Snapshot of the most recent compaction event (for UI "last compacted
    /// at..." indicators).
    pub fn last_event(&self) -> Option<CompactionEvent> { self.last_event.lock().clone() }

    /// Current usage.
    pub fn usage(&self) -> TokenUsage { self.state.lock().usage.clone() }

    /// Drop all conversation/history segments; keep the structural skeleton
    /// (system, objective, plan, constraint, decision). Used for emergency
    /// compaction before handoff to another agent.
    pub fn keep_skeleton_only(&self) {
        let mut s = self.state.lock();
        s.segments.retain(|seg| is_skeleton(seg));
        s.usage.used = s.segments.iter().map(|seg| seg.tokens).sum();
    }
}

fn is_skeleton(seg: &ContextSegment) -> bool {
    matches!(
        seg.kind,
        crate::state::ContextSegmentKind::System
            | crate::state::ContextSegmentKind::Objective
            | crate::state::ContextSegmentKind::Plan
            | crate::state::ContextSegmentKind::Constraint
            | crate::state::ContextSegmentKind::UserRequirement
            | crate::state::ContextSegmentKind::Decision
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compaction::CompactionTrigger;
    use crate::state::{ContextSegment, ContextSegmentKind, SegmentMeta};

    #[test]
    fn pressure_thresholds_routes_correctly() {
        let mut cfg = ContextManagerConfig::new("a", "m", 100);
        cfg.thresholds.warn = 0.5;
        cfg.thresholds.compact = 0.7;
        cfg.thresholds.emergency = 0.9;
        let m = ContextManager::new(cfg);
        for i in 0..10 {
            m.push(ContextSegment {
                kind: ContextSegmentKind::Conversation,
                title: format!("c{i}"),
                body: "x".repeat(40),
                tokens: 10,
                meta: SegmentMeta::new(format!("c-{i}")),
            });
        }
        let action = m.pressure();
        assert_eq!(action, ThresholdAction::Emergency);
    }

    #[test]
    fn force_compact_emits_event() {
        let m = ContextManager::new(ContextManagerConfig::new("a", "m", 1000));
        let ev = m.force_compact("s", CompactionTrigger::Manual);
        assert_eq!(ev.trigger, CompactionTrigger::Manual);
    }
}
