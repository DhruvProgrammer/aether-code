//! Agent lifecycle (spec §34-§35, §36). Track run state and parent/child relationships, and
//! mechanically enforce recursion depth and child counts to prevent agent explosions.

use chrono::Utc;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Created,
    Queued,
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
    Blocked,
}

impl AgentStatus {
    pub fn label(&self) -> &'static str {
        match self {
            AgentStatus::Created => "CREATED",
            AgentStatus::Queued => "QUEUED",
            AgentStatus::Running => "RUNNING",
            AgentStatus::Waiting => "WAITING",
            AgentStatus::Completed => "COMPLETED",
            AgentStatus::Failed => "FAILED",
            AgentStatus::Cancelled => "CANCELLED",
            AgentStatus::TimedOut => "TIMED_OUT",
            AgentStatus::Blocked => "BLOCKED",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentRun {
    pub run_id: String,
    pub agent_id: String,
    pub parent_run_id: Option<String>,
    pub session_id: String,
    pub task_id: Option<String>,
    pub loop_run_id: Option<String>,
    pub status: AgentStatus,
    pub started_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Default)]
pub struct LifecycleTracker {
    depth: usize,
    max_depth: usize,
    children: HashMap<String, usize>,
    max_children: usize,
}

impl LifecycleTracker {
    pub fn new(max_depth: usize, max_children: usize) -> Self {
        Self {
            depth: 0,
            max_depth: max_depth.max(1),
            children: HashMap::new(),
            max_children: max_children.max(1),
        }
    }

    /// Whether a child may be spawned under `parent_run_id` without violating depth/child limits.
    pub fn can_spawn(&self, parent_run_id: Option<&str>) -> bool {
        if self.depth + 1 > self.max_depth {
            return false;
        }
        match parent_run_id {
            Some(p) => self.children.get(p).copied().unwrap_or(0) < self.max_children,
            None => true,
        }
    }

    /// Record a new run in the RUNNING state (also increments depth/child counters).
    pub fn start(
        &mut self,
        agent_id: &str,
        parent_run_id: Option<&str>,
        session_id: &str,
        task_id: Option<&str>,
        loop_run_id: Option<&str>,
    ) -> AgentRun {
        if parent_run_id.is_some() {
            self.depth += 1;
            if let Some(p) = parent_run_id {
                *self.children.entry(p.to_string()).or_insert(0) += 1;
            }
        }
        AgentRun {
            run_id: format!("{}.{}.{}", session_id, agent_id, Utc::now().timestamp_millis()),
            agent_id: agent_id.to_string(),
            parent_run_id: parent_run_id.map(str::to_string),
            session_id: session_id.to_string(),
            task_id: task_id.map(str::to_string),
            loop_run_id: loop_run_id.map(str::to_string),
            status: AgentStatus::Running,
            started_at: Utc::now().to_rfc3339(),
            ended_at: None,
        }
    }

    /// Transition a run to a terminal/non-running state and unwind depth.
    pub fn finish(&mut self, run: &mut AgentRun, status: AgentStatus) {
        run.status = status;
        run.ended_at = Some(Utc::now().to_rfc3339());
        if run.parent_run_id.is_some() && self.depth > 0 {
            self.depth -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_and_child_limits() {
        let mut t = LifecycleTracker::new(2, 2);
        assert!(t.can_spawn(None));
        let parent = t.start("explorer", None, "s", None, None);
        assert!(t.can_spawn(Some(&parent.run_id)));
        let _c1 = t.start("planner", Some(&parent.run_id), "s", None, None);
        let _c2 = t.start("designer", Some(&parent.run_id), "s", None, None);
        // third child beyond max_children=2
        assert!(!t.can_spawn(Some(&parent.run_id)));
    }

    #[test]
    fn finish_unwinds_depth() {
        let mut t = LifecycleTracker::new(3, 5);
        let mut run = t.start("explorer", None, "s", None, None);
        assert_eq!(t.depth, 0); // top-level has no parent
        t.finish(&mut run, AgentStatus::Completed);
        assert_eq!(run.status, AgentStatus::Completed);
        assert!(run.ended_at.is_some());
    }
}
