//! Agent-aware context workspace (v0.13).
//!
//! OpenCode holds one global session context. AETHER's 3-LLM hierarchy needs
//! **per-agent contexts** that compact independently, plus a **shared global
//! layer** of promoted segments that every agent can consult without paying
//! the cost of a full conversation replay.
//!
//! Layout:
//!
//! ```text
//!                    ContextWorkspace (global)
//!                             │
//!        ┌────────────────────┼────────────────────┐
//!        │                    │                    │
//!   SharedSegments      Controller ctx        Specialist ctx (per agent)
//!   (objective, plan,   (independent          (independent compaction,
//!    decisions,          compaction)           isolated state)
//!    constraints,
//!    evidence)
//! ```
//!
//! * Each agent (controller, coder, tester, reviewer, …) owns its own
//!   [`ContextManager`]. Compacting the coder's context does not touch the
//!   controller's.
//! * Structurally important information is **promoted** to the workspace via
//!   [`ContextWorkspace::promote`] and becomes a *shared segment*. Shared
//!   segments are pinned and are injected into every agent's view via
//!   [`ContextWorkspace::shared_block`].
//! * [`ContextWorkspace::fleet_status`] reports usage across all agents so the
//!   UI can render per-agent context pressure.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::manager::{ContextManager, ContextManagerConfig};
use crate::state::{ContextSegment, ContextSegmentKind};

/// A segment promoted from an agent into the shared global layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedSegment {
    pub id: String,
    pub kind: ContextSegmentKind,
    pub content: String,
    pub tokens: u32,
    /// Which agent promoted it.
    pub promoted_by: String,
    pub pinned: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl SharedSegment {
    pub fn new(
        kind: ContextSegmentKind,
        content: impl Into<String>,
        tokens: u32,
        promoted_by: impl Into<String>,
    ) -> Self {
        let content = content.into();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            content,
            tokens,
            promoted_by: promoted_by.into(),
            pinned: true,
            created_at: chrono::Utc::now(),
        }
    }
}

/// Per-agent context pressure report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContextStatus {
    pub agent: String,
    pub model: String,
    pub context_window: u32,
    pub used_tokens: u32,
    pub usage_pct: f32,
    pub segment_count: usize,
    pub pressure: String,
}

/// Fleet of per-agent context managers + a shared segment layer.
#[derive(Default)]
pub struct ContextWorkspace {
    managers: RwLock<HashMap<String, Arc<ContextManager>>>,
    shared: RwLock<Vec<SharedSegment>>,
}

impl ContextWorkspace {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- Per-agent managers ----------------------------------------------

    /// Get or create the `ContextManager` for an agent. The context window is
    /// model-specific; callers pass it in on creation.
    pub fn manager_for(&self, agent: &str, model: &str, context_window: u32) -> Arc<ContextManager> {
        {
            let m = self.managers.read();
            if let Some(existing) = m.get(agent) {
                return existing.clone();
            }
        }
        let mut m = self.managers.write();
        m.entry(agent.to_string())
            .or_insert_with(|| {
                Arc::new(ContextManager::new(ContextManagerConfig::new(
                    agent,
                    model,
                    context_window,
                )))
            })
            .clone()
    }

    /// Get the manager for an agent if it exists, else `None`.
    pub fn get(&self, agent: &str) -> Option<Arc<ContextManager>> {
        self.managers.read().get(agent).cloned()
    }

    /// Drop an agent's context (e.g. when a specialist completes). Shared
    /// segments are untouched.
    pub fn drop_agent(&self, agent: &str) {
        self.managers.write().remove(agent);
    }

    /// All agent ids currently tracked.
    pub fn agents(&self) -> Vec<String> {
        let mut v: Vec<String> = self.managers.read().keys().cloned().collect();
        v.sort();
        v
    }

    // ---- Shared segment layer ---------------------------------------------

    /// Promote a segment from an agent into the shared layer. Every agent can
    /// see the shared layer via [`shared_block`]. Kinds intended for global
    /// reach: Objective, Plan, Decision, Constraint, AgentFinding,
    /// UserRequirement, UnresolvedQuestion.
    pub fn promote(&self, seg: SharedSegment) -> String {
        self.shared.write().push(seg.clone());
        seg.id.clone()
    }

    pub fn shared(&self) -> Vec<SharedSegment> {
        self.shared.read().clone()
    }

    /// Render the shared layer as a single text block that can be injected at
    /// the top of any agent's system prompt.
    pub fn shared_block(&self) -> String {
        let segs = self.shared.read();
        if segs.is_empty() {
            return String::new();
        }
        let mut out = String::from("# Shared Global Context\n\n");
        for s in segs.iter() {
            out.push_str(&format!("[{:?} by {}] {}\n", s.kind, s.promoted_by, s.content));
        }
        out
    }

    /// Total shared tokens (for budgeting against each agent's window).
    pub fn shared_tokens(&self) -> u32 {
        self.shared.read().iter().map(|s| s.tokens).sum()
    }

    /// Remove a shared segment by id (e.g. an obsolete plan superseded by a
    /// new one).
    pub fn demote(&self, id: &str) -> bool {
        let before = self.shared.read().len();
        self.shared.write().retain(|s| s.id != id);
        self.shared.read().len() < before
    }

    // ---- Fleet status ------------------------------------------------------

    /// Usage report across every tracked agent.
    pub fn fleet_status(&self) -> Vec<AgentContextStatus> {
        self.managers
            .read()
            .iter()
            .map(|(agent, m)| {
                let snap = m.snapshot();
                AgentContextStatus {
                    agent: agent.clone(),
                    model: m.model().to_string(),
                    context_window: m.config().context_window,
                    used_tokens: snap.usage.used,
                    usage_pct: snap.usage.pct(),
                    segment_count: snap.segments.len(),
                    pressure: format!("{:?}", m.pressure()),
                }
            })
            .collect()
    }

    /// The agent under the most context pressure, if any.
    pub fn most_pressured(&self) -> Option<AgentContextStatus> {
        let mut v = self.fleet_status();
        v.sort_by(|a, b| b.usage_pct.partial_cmp(&a.usage_pct).unwrap_or(std::cmp::Ordering::Equal));
        v.into_iter().next()
    }

    // ---- Compaction --------------------------------------------------------

    /// Run threshold-based compaction on ONE agent only. Independent compaction
    /// is the whole point of the workspace: the coder can compact its huge
    /// tool-output trail without touching the controller's plan state.
    ///
    /// Returns the compaction event if compaction ran.
    pub fn compact_agent(&self, agent: &str, session_id: &str) -> Option<crate::events::CompactionEvent> {
        let m = self.managers.read().get(agent).cloned()?;
        m.check_and_compact(session_id)
    }

    /// Run threshold-based compaction across every agent above its threshold.
    /// Returns events for every agent that compacted.
    pub fn compact_all(
        &self,
        session_id: &str,
    ) -> Vec<(String, crate::events::CompactionEvent)> {
        let mgrs: Vec<(String, Arc<ContextManager>)> =
            self.managers.read().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        mgrs.into_iter()
            .filter_map(|(agent, m)| {
                m.check_and_compact(session_id).map(|e| (agent, e))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ContextSegment, ContextSegmentKind, SegmentMeta};

    fn seg(agent: &str, tokens: u32) -> ContextSegment {
        ContextSegment {
            kind: ContextSegmentKind::Conversation,
            title: format!("{agent} segment"),
            body: "x".repeat(tokens as usize),
            tokens,
            meta: SegmentMeta::new(format!("seg-{agent}")),
        }
    }

    #[test]
    fn managers_are_independent() {
        let ws = ContextWorkspace::new();
        let controller = ws.manager_for("controller", "model-a", 100_000);
        let coder = ws.manager_for("coder", "model-b", 32_000);
        controller.push(seg("controller", 1_000));
        coder.push(seg("coder", 10_000));
        let status = ws.fleet_status();
        let c = status.iter().find(|s| s.agent == "controller").unwrap();
        let cod = status.iter().find(|s| s.agent == "coder").unwrap();
        assert_eq!(c.used_tokens, 1_000);
        assert_eq!(cod.used_tokens, 10_000);
        assert!(cod.usage_pct > c.usage_pct);
    }

    #[test]
    fn same_agent_returns_same_manager() {
        let ws = ContextWorkspace::new();
        let a = ws.manager_for("controller", "m", 1000);
        let b = ws.manager_for("controller", "m", 1000);
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn shared_block_lists_promoted_segments() {
        let ws = ContextWorkspace::new();
        ws.promote(SharedSegment::new(
            ContextSegmentKind::Objective,
            "Build OAuth login",
            10,
            "controller",
        ));
        let block = ws.shared_block();
        assert!(block.contains("Shared Global Context"));
        assert!(block.contains("OAuth login"));
        assert!(block.contains("controller"));
        assert_eq!(ws.shared_tokens(), 10);
    }

    #[test]
    fn demote_removes_segment() {
        let ws = ContextWorkspace::new();
        let id = ws.promote(SharedSegment::new(
            ContextSegmentKind::Plan,
            "old plan",
            5,
            "controller",
        ));
        assert!(ws.demote(&id));
        assert!(ws.shared().is_empty());
        assert!(!ws.demote("nonexistent"));
    }

    #[test]
    fn drop_agent_isolates_from_shared() {
        let ws = ContextWorkspace::new();
        let _ = ws.manager_for("coder", "m", 1000);
        ws.promote(SharedSegment::new(
            ContextSegmentKind::Decision,
            "Use JWT",
            3,
            "coder",
        ));
        ws.drop_agent("coder");
        assert!(ws.get("coder").is_none());
        // Shared layer survives agent drop.
        assert_eq!(ws.shared().len(), 1);
    }

    #[test]
    fn compact_agent_only_touches_that_agent() {
        let ws = ContextWorkspace::new();
        // Tiny window so pressure is high for the coder only.
        let coder = ws.manager_for("coder", "m", 100);
        let controller = ws.manager_for("controller", "m", 100_000);
        coder.push(seg("coder", 90));
        controller.push(seg("controller", 10));
        let events = ws.compact_all("s1");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "coder");
    }

    #[test]
    fn most_pressured_is_max_usage() {
        let ws = ContextWorkspace::new();
        let coder = ws.manager_for("coder", "m", 1_000);
        let _controller = ws.manager_for("controller", "m", 1_000_000);
        coder.push(seg("coder", 900));
        let top = ws.most_pressured().unwrap();
        assert_eq!(top.agent, "coder");
    }
}
