//! Compaction events + sinks.

use crate::compaction::CompactionTrigger;
use crate::state::ContextSegmentKind;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// What happened during a compaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionEvent {
    pub session_id: String,
    pub agent: String,
    pub trigger: CompactionTrigger,
    pub tokens_before: u32,
    pub tokens_after: u32,
    pub segments_kept: u32,
    pub segments_summarised: u32,
    pub segments_dropped: u32,
    /// Kind-level summary: what kinds of segment survived vs were summarised.
    pub preservation: Vec<PreservationRecord>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreservationRecord {
    pub kind: ContextSegmentKind,
    pub action: PreservationAction,
    pub segments: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreservationAction {
    Preserved,
    Summarised,
    Dropped,
}

/// Anything that wants to hear about context lifecycle events.
pub trait ContextEventSink: Send + Sync {
    fn on_compaction(&self, ev: &CompactionEvent);
}

/// Sink that drops everything. Default for tests.
#[derive(Debug, Clone, Default)]
pub struct NullSink;
impl ContextEventSink for NullSink {
    fn on_compaction(&self, _ev: &CompactionEvent) {}
}

/// Multi-cast sink that forwards to every inner sink.
#[derive(Clone, Default)]
pub struct FanOutSink {
    sinks: Vec<Arc<dyn ContextEventSink>>,
}

impl FanOutSink {
    pub fn new() -> Self { Self::default() }
    pub fn push(&mut self, s: Arc<dyn ContextEventSink>) { self.sinks.push(s); }
}

impl ContextEventSink for FanOutSink {
    fn on_compaction(&self, ev: &CompactionEvent) {
        for s in &self.sinks {
            s.on_compaction(ev);
        }
    }
}
