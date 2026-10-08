//! Structured harness runtime events (spec §23).
//!
//! AETHER already ships four event mechanisms (a typed `TaskEventKind` sink,
//! a `RuntimeEventBus`, the plugin bus, and the context sink). The harness
//! does **not** add a fifth transport: it emits its own typed events here and
//! the caller projects them into the existing `TaskEventKind` sink or the
//! runtime bus. This keeps the harness honest about non-duplication.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HarnessEvent {
    SessionStarted {
        session_id: String,
    },
    SessionResumed {
        session_id: String,
        restored_from: String,
    },
    GoalCreated {
        goal_id: String,
        objective: String,
    },
    GoalUpdated {
        goal_id: String,
        status: String,
        progress: String,
    },
    TaskStarted {
        task_id: String,
        title: String,
    },
    TaskCompleted {
        task_id: String,
        outcome: String,
    },
    SubagentSpawned {
        child_id: String,
        parent_id: Option<String>,
        agent_id: String,
        depth: usize,
    },
    SubagentCompleted {
        child_id: String,
        status: String,
        summary: String,
    },
    SubagentFailed {
        child_id: String,
        reason: String,
    },
    MemoryUpdated {
        inserted: usize,
    },
    RefinementCreated {
        refinement_id: String,
        kind: String,
    },
    RefinementApplied {
        refinement_id: String,
        edits_applied: usize,
        edits_failed: usize,
    },
    CompactionStarted {
        trigger: String,
    },
    CompactionCompleted {
        tokens_before: u32,
        tokens_after: u32,
    },
    Heartbeat {
        session_id: String,
        reason: String,
    },
    AutonomousStarted {
        goal_id: Option<String>,
    },
    AutonomousPaused {
        reason: String,
    },
    AutonomousCompleted {
        reason: String,
        /// Explicitly false when execution stopped because a budget was
        /// exhausted rather than because the goal was met (spec §17).
        success: bool,
    },
    GateFailed {
        command: String,
        attempt: u32,
        exit: String,
    },
    RecoveryRestored {
        session_id: String,
        detail: String,
    },
    RefinementRejected {
        reason: String,
    },
}

/// Callback the runtime injects (mirrors `Agent::with_task_event_sink`).
pub type HarnessSink = Arc<dyn Fn(HarnessEvent) + Send + Sync>;

/// A sink that drops everything — the default when nobody is listening.
pub fn null_sink() -> HarnessSink {
    Arc::new(|_| {})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn sink_receives_events() {
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        let sink: HarnessSink = Arc::new(move |_| {
            n2.fetch_add(1, Ordering::Relaxed);
        });
        sink(HarnessEvent::SessionStarted {
            session_id: "s".into(),
        });
        sink(HarnessEvent::Heartbeat {
            session_id: "s".into(),
            reason: "children settled".into(),
        });
        assert_eq!(n.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn events_are_serializable_for_ui() {
        let ev = HarnessEvent::AutonomousCompleted {
            reason: "maxTurns".into(),
            success: false,
        };
        let j = serde_json::to_string(&ev).unwrap();
        assert!(j.contains("\"event\":\"autonomous_completed\""));
        assert!(j.contains("\"success\":false"));
        let back: HarnessEvent = serde_json::from_str(&j).unwrap();
        match back {
            HarnessEvent::AutonomousCompleted { success, .. } => assert!(!success),
            _ => panic!("wrong variant"),
        }
    }
}
