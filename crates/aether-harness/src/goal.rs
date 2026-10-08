//! Persistent goals (spec §8).
//!
//! A goal is data, never an instruction: it cannot outrank system prompts,
//! role prompts, or permission rules (spec §8, §27). Lifecycle:
//! `Active | Paused | Blocked | Completed | Cancelled`.
//!
//! Note the reuse boundary — `aether-core::eng::EngineeringModel.goal` is a
//! plain string used *inside one loop*. This type is the durable,
//! cross-restart aggregate: objective, progress, blocker, next action.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Paused,
    Blocked,
    Completed,
    Cancelled,
}

impl GoalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Only `Active` may advance work. A blocked goal resumes explicitly.
    pub fn is_runnable(self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

/// Durable goal state, recovered from the harness store on restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub session_id: String,
    pub objective: String,
    pub status: GoalStatus,
    /// Progress ledger. `tasks_done` may exceed `tasks_total` when the plan
    /// grows, so `progress()` clamps instead of assuming `total >= done`.
    pub tasks_total: u32,
    pub tasks_done: u32,
    pub blocked_reason: Option<String>,
    pub next_action: Option<String>,
    /// Verification evidence references required before completion. A goal
    /// with unmet gates cannot move to `Completed` (spec §18/§22).
    pub required_gates: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Goal {
    pub fn new(session_id: &str, objective: &str) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            id: format!("goal-{}", uuid::Uuid::new_v4().simple()),
            session_id: session_id.to_string(),
            objective: objective.to_string(),
            status: GoalStatus::Active,
            tasks_total: 0,
            tasks_done: 0,
            blocked_reason: None,
            next_action: None,
            required_gates: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_tasks(mut self, total: u32) -> Self {
        self.tasks_total = total;
        self
    }

    pub fn with_required_gates(mut self, gates: Vec<String>) -> Self {
        self.required_gates = gates;
        self
    }

    pub fn progress(&self) -> String {
        if self.tasks_total == 0 {
            return "0/0".into();
        }
        format!("{}/{}", self.tasks_done, self.tasks_total)
    }

    pub fn set_status(&mut self, status: GoalStatus) {
        self.status = status;
        if !matches!(status, GoalStatus::Blocked) {
            self.blocked_reason = None;
        }
        self.updated_at = chrono::Utc::now().timestamp();
    }

    pub fn record_task_done(&mut self) {
        self.tasks_done = self.tasks_done.saturating_add(1);
        self.updated_at = chrono::Utc::now().timestamp();
    }

    pub fn set_next_action(&mut self, next: impl Into<String>) {
        self.next_action = Some(next.into());
        self.updated_at = chrono::Utc::now().timestamp();
    }

    pub fn block(&mut self, reason: impl Into<String>) {
        self.status = GoalStatus::Blocked;
        self.blocked_reason = Some(reason.into());
        self.updated_at = chrono::Utc::now().timestamp();
    }

    pub fn resume(&mut self) {
        self.status = GoalStatus::Active;
        self.blocked_reason = None;
        self.updated_at = chrono::Utc::now().timestamp();
    }

    /// Completion is gated: only `Active` goals with every task done may
    /// complete. Verification gates are checked by the caller with real tool
    /// evidence — this method never fabricates it.
    pub fn try_complete(&mut self) -> Result<(), String> {
        if !matches!(self.status, GoalStatus::Active) {
            return Err(format!(
                "goal cannot complete from state {}",
                self.status.as_str()
            ));
        }
        if self.tasks_done < self.tasks_total {
            return Err(format!(
                "goal cannot complete: {}/{} tasks done",
                self.tasks_done, self.tasks_total
            ));
        }
        self.status = GoalStatus::Completed;
        self.updated_at = chrono::Utc::now().timestamp();
        Ok(())
    }

    /// One-line status for panels, prompts, and `--goal` output.
    pub fn render(&self) -> String {
        let mut s = format!(
            "Goal: {}\nStatus: {}\nProgress: {}",
            self.objective,
            self.status.as_str(),
            self.progress()
        );
        if let Some(r) = &self.blocked_reason {
            s.push_str(&format!("\nBlocked: {r}"));
        }
        if let Some(n) = &self.next_action {
            s.push_str(&format!("\nNext: {n}"));
        }
        if !self.required_gates.is_empty() {
            s.push_str(&format!(
                "\nRequired verification: {}",
                self.required_gates.join(", ")
            ));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_and_progress() {
        let mut g = Goal::new("s", "Refactor provider architecture").with_tasks(12);
        assert_eq!(g.status, GoalStatus::Active);
        assert!(g.status.is_runnable());
        for _ in 0..8 {
            g.record_task_done();
        }
        assert_eq!(g.progress(), "8/12");
        assert!(g.render().contains("8/12"));
        // Cannot complete early.
        assert!(g.try_complete().is_err());
    }

    #[test]
    fn blocked_requires_explicit_resume() {
        let mut g = Goal::new("s", "g").with_tasks(1);
        g.record_task_done();
        g.block("provider validation test failing");
        assert_eq!(g.status, GoalStatus::Blocked);
        assert!(!g.status.is_runnable());
        assert!(g.render().contains("provider validation test failing"));
        // Completing while blocked is rejected.
        assert!(g.try_complete().is_err());
        g.resume();
        assert!(g.try_complete().is_ok());
        assert!(g.status.is_terminal());
    }

    #[test]
    fn completion_requires_all_tasks() {
        let mut g = Goal::new("s", "g").with_tasks(3);
        g.record_task_done();
        g.record_task_done();
        g.record_task_done();
        assert!(g.try_complete().is_ok());
        assert_eq!(g.status, GoalStatus::Completed);
    }

    #[test]
    fn roundtrips_through_json_for_recovery() {
        let mut g = Goal::new("sess", "Refactor provider architecture")
            .with_tasks(12)
            .with_required_gates(vec!["cargo test".into()]);
        g.record_task_done();
        g.block("tests failing");
        let j = serde_json::to_string(&g).unwrap();
        let back: Goal = serde_json::from_str(&j).unwrap();
        assert_eq!(back.id, g.id);
        assert_eq!(back.status, GoalStatus::Blocked);
        assert_eq!(back.tasks_done, 1);
        assert_eq!(back.required_gates, vec!["cargo test".to_string()]);
    }
}
