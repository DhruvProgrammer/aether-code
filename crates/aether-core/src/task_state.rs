//! Authoritative 3-LLM Task State Machine (spec: 3-LLM Real Task State Machine).
//!
//! One state machine governs the full lifecycle of every AETHER task:
//!
//! ```text
//! CREATED → UNDERSTANDING → PLANNING → PLAN_READY → EXECUTING
//!   → REVIEWING → VERIFYING → COMPLETED
//!   → (on failure) REPLANNING → REPAIRING → REVIEWING → VERIFYING
//! ```
//!
//! Roles:
//!   * LLM 1 — Executor/Builder (implements, repairs, runs tools)
//!   * LLM 2 — Planner/Orchestrator (plans, replans, decomposes)
//!   * LLM 3 — Observer/Reviewer/Verifier (understands, reviews, verifies, concludes)
//!
//! The state machine never changes model assignments, never bypasses
//! permissions, and never allows LLM 1 to declare task completion.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn now() -> String {
    Utc::now().to_rfc3339()
}

// ---------------------------------------------------------------------------
// States
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskState {
    Created,
    Understanding,
    Planning,
    PlanReady,
    Executing,
    Reviewing,
    Verifying,
    Replanning,
    Repairing,
    WaitingUser,
    WaitingTool,
    WaitingNetwork,
    Blocked,
    Completed,
    Failed,
    Cancelled,
}

impl TaskState {
    pub fn label(&self) -> &'static str {
        match self {
            TaskState::Created => "CREATED",
            TaskState::Understanding => "UNDERSTANDING",
            TaskState::Planning => "PLANNING",
            TaskState::PlanReady => "PLAN_READY",
            TaskState::Executing => "EXECUTING",
            TaskState::Reviewing => "REVIEWING",
            TaskState::Verifying => "VERIFYING",
            TaskState::Replanning => "REPLANNING",
            TaskState::Repairing => "REPAIRING",
            TaskState::WaitingUser => "WAITING_USER",
            TaskState::WaitingTool => "WAITING_TOOL",
            TaskState::WaitingNetwork => "WAITING_NETWORK",
            TaskState::Blocked => "BLOCKED",
            TaskState::Completed => "COMPLETED",
            TaskState::Failed => "FAILED",
            TaskState::Cancelled => "CANCELLED",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, TaskState::Completed | TaskState::Failed | TaskState::Cancelled)
    }

    pub fn is_waiting(&self) -> bool {
        matches!(
            self,
            TaskState::WaitingUser | TaskState::WaitingTool | TaskState::WaitingNetwork | TaskState::Blocked
        )
    }

    pub fn is_active_work(&self) -> bool {
        matches!(
            self,
            TaskState::Understanding
                | TaskState::Planning
                | TaskState::Executing
                | TaskState::Reviewing
                | TaskState::Verifying
                | TaskState::Replanning
                | TaskState::Repairing
        )
    }
}

// ---------------------------------------------------------------------------
// Roles
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmRole {
    System,
    Executor,
    Planner,
    Reviewer,
}

impl LlmRole {
    pub fn label(&self) -> &'static str {
        match self {
            LlmRole::System => "System",
            LlmRole::Executor => "LLM 1 — Executor",
            LlmRole::Planner => "LLM 2 — Planner",
            LlmRole::Reviewer => "LLM 3 — Reviewer",
        }
    }
}

// ---------------------------------------------------------------------------
// Transition history
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionRecord {
    pub from_state: TaskState,
    pub to_state: TaskState,
    pub active_role: LlmRole,
    pub reason: String,
    pub timestamp: String,
    pub operation_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Verification evidence
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvidenceItem {
    pub kind: String,
    pub status: String,
    pub detail: String,
    pub output_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VerificationEvidence {
    pub items: Vec<EvidenceItem>,
    pub overall_pass: bool,
    pub concluded_by: Option<LlmRole>,
    pub timestamp: Option<String>,
}

impl VerificationEvidence {
    pub fn has_evidence(&self) -> bool {
        !self.items.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Structured handoffs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handoff {
    pub task_id: String,
    pub session_id: String,
    pub from_role: LlmRole,
    pub to_role: LlmRole,
    pub objective: String,
    pub current_state: TaskState,
    pub plan_summary: Option<String>,
    pub review_status: Option<String>,
    pub findings: Vec<String>,
    pub relevant_files: Vec<String>,
    pub important_decisions: Vec<String>,
    pub tool_result_refs: Vec<String>,
    pub required_action: String,
    pub timestamp: String,
}

// ---------------------------------------------------------------------------
// Task record (authoritative state)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub task_id: String,
    pub session_id: String,
    pub parent_task_id: Option<String>,

    pub state: TaskState,
    pub previous_state: Option<TaskState>,

    pub active_role: LlmRole,
    pub current_activity: String,

    pub current_plan: Option<String>,
    pub current_plan_step: Option<u32>,
    pub total_plan_steps: Option<u32>,
    pub completed_steps: Vec<String>,
    pub pending_steps: Vec<String>,
    pub blocked_steps: Vec<String>,

    pub attempt_count: u32,
    pub repair_attempt_count: u32,
    pub replan_count: u32,
    pub verification_attempt_count: u32,

    pub active_tool: Option<String>,
    pub active_operation: Option<String>,
    pub active_operation_id: Option<String>,

    pub last_tool_result: Option<String>,
    pub last_error: Option<String>,
    pub failure_category: Option<String>,

    pub verification: VerificationEvidence,

    pub relevant_files: Vec<String>,
    pub important_decisions: Vec<String>,

    pub next_action: Option<String>,

    pub created_at: String,
    pub started_at: Option<String>,
    pub updated_at: String,
    pub completed_at: Option<String>,

    pub checkpoint_reference: Option<String>,

    pub transitions: Vec<TransitionRecord>,
}

// ---------------------------------------------------------------------------
// Doom-loop detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DoomLoopTracker {
    pub failure_fingerprints: Vec<String>,
    pub strategy_fingerprints: Vec<String>,
    pub changed_file_fingerprints: Vec<String>,
    pub max_repair_attempts: u32,
    pub max_replans: u32,
    pub doom_detected: bool,
    pub doom_reason: Option<String>,
}

impl DoomLoopTracker {
    pub fn new(max_repair: u32, max_replans: u32) -> Self {
        Self {
            max_repair_attempts: max_repair.max(1),
            max_replans: max_replans.max(1),
            ..Default::default()
        }
    }

    pub fn record_failure(&mut self, fingerprint: &str) {
        self.failure_fingerprints.push(fingerprint.to_string());
        if self.failure_fingerprints.len() > 20 {
            let excess = self.failure_fingerprints.len() - 20;
            self.failure_fingerprints.drain(0..excess);
        }
        self.check_doom();
    }

    pub fn record_strategy(&mut self, fingerprint: &str) {
        self.strategy_fingerprints.push(fingerprint.to_string());
        if self.strategy_fingerprints.len() > 20 {
            let excess = self.strategy_fingerprints.len() - 20;
            self.strategy_fingerprints.drain(0..excess);
        }
        self.check_doom();
    }

    pub fn record_changed_files(&mut self, fingerprint: &str) {
        self.changed_file_fingerprints.push(fingerprint.to_string());
        if self.changed_file_fingerprints.len() > 20 {
            let excess = self.changed_file_fingerprints.len() - 20;
            self.changed_file_fingerprints.drain(0..excess);
        }
        self.check_doom();
    }

    fn check_doom(&mut self) {
        if self.doom_detected {
            return;
        }
        if has_repeated(&self.failure_fingerprints, 3) {
            self.doom_detected = true;
            self.doom_reason = Some("same failure fingerprint repeated 3+ times".into());
        } else if has_repeated(&self.strategy_fingerprints, 3) {
            self.doom_detected = true;
            self.doom_reason = Some("same strategy fingerprint repeated 3+ times".into());
        } else if has_repeated(&self.changed_file_fingerprints, 4) {
            self.doom_detected = true;
            self.doom_reason = Some("same changed-file set repeated 4+ times".into());
        }
    }
}

fn has_repeated(items: &[String], threshold: usize) -> bool {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for item in items {
        *counts.entry(item.as_str()).or_insert(0) += 1;
    }
    counts.values().any(|&c| c >= threshold)
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

pub struct TaskStateMachine {
    pub record: TaskRecord,
    pub doom: DoomLoopTracker,
    resume_state: Option<TaskState>,
}

impl TaskStateMachine {
    pub fn new(task_id: &str, session_id: &str) -> Self {
        let ts = now();
        Self {
            record: TaskRecord {
                task_id: task_id.to_string(),
                session_id: session_id.to_string(),
                parent_task_id: None,
                state: TaskState::Created,
                previous_state: None,
                active_role: LlmRole::System,
                current_activity: String::new(),
                current_plan: None,
                current_plan_step: None,
                total_plan_steps: None,
                completed_steps: Vec::new(),
                pending_steps: Vec::new(),
                blocked_steps: Vec::new(),
                attempt_count: 0,
                repair_attempt_count: 0,
                replan_count: 0,
                verification_attempt_count: 0,
                active_tool: None,
                active_operation: None,
                active_operation_id: None,
                last_tool_result: None,
                last_error: None,
                failure_category: None,
                verification: VerificationEvidence::default(),
                relevant_files: Vec::new(),
                important_decisions: Vec::new(),
                next_action: None,
                created_at: ts.clone(),
                started_at: None,
                updated_at: ts,
                completed_at: None,
                checkpoint_reference: None,
                transitions: Vec::new(),
            },
            doom: DoomLoopTracker::new(3, 3),
            resume_state: None,
        }
    }

    pub fn with_parent(mut self, parent_task_id: &str) -> Self {
        self.record.parent_task_id = Some(parent_task_id.to_string());
        self
    }

    pub fn with_limits(mut self, max_repair: u32, max_replans: u32) -> Self {
        self.doom = DoomLoopTracker::new(max_repair, max_replans);
        self
    }

    pub fn state(&self) -> TaskState {
        self.record.state
    }

    pub fn active_role(&self) -> LlmRole {
        self.record.active_role
    }

    fn validate_transition(from: TaskState, to: TaskState) -> bool {
        if from == to {
            return false;
        }
        if from.is_terminal() {
            return false;
        }
        use TaskState::*;
        match (from, to) {
            (Created, Understanding) => true,
            (Created, Cancelled) => true,
            (Understanding, Planning) => true,
            (Understanding, WaitingUser) => true,
            (Understanding, Cancelled) => true,
            (Understanding, Failed) => true,
            (Planning, PlanReady) => true,
            (Planning, WaitingUser) => true,
            (Planning, Cancelled) => true,
            (Planning, Failed) => true,
            (PlanReady, Executing) => true,
            (PlanReady, Replanning) => true,
            (PlanReady, Cancelled) => true,
            (Executing, Reviewing) => true,
            (Executing, WaitingTool) => true,
            (Executing, WaitingNetwork) => true,
            (Executing, WaitingUser) => true,
            (Executing, Blocked) => true,
            (Executing, Cancelled) => true,
            (Executing, Failed) => true,
            (Reviewing, Verifying) => true,
            (Reviewing, Replanning) => true,
            (Reviewing, Cancelled) => true,
            (Reviewing, Failed) => true,
            (Verifying, Completed) => true,
            (Verifying, Replanning) => true,
            (Verifying, Cancelled) => true,
            (Verifying, Failed) => true,
            (Replanning, Repairing) => true,
            (Replanning, PlanReady) => true,
            (Replanning, Cancelled) => true,
            (Replanning, Failed) => true,
            (Repairing, Reviewing) => true,
            (Repairing, WaitingTool) => true,
            (Repairing, Cancelled) => true,
            (Repairing, Failed) => true,
            (WaitingUser, Understanding) => true,
            (WaitingUser, Planning) => true,
            (WaitingUser, Executing) => true,
            (WaitingUser, Replanning) => true,
            (WaitingUser, Cancelled) => true,
            (WaitingUser, Failed) => true,
            (WaitingTool, Executing) => true,
            (WaitingTool, Repairing) => true,
            (WaitingTool, Cancelled) => true,
            (WaitingTool, Failed) => true,
            (WaitingNetwork, Executing) => true,
            (WaitingNetwork, Repairing) => true,
            (WaitingNetwork, Cancelled) => true,
            (WaitingNetwork, Failed) => true,
            (Blocked, Replanning) => true,
            (Blocked, WaitingUser) => true,
            (Blocked, Cancelled) => true,
            (Blocked, Failed) => true,
            _ => false,
        }
    }

    pub fn transition(&mut self, to: TaskState, role: LlmRole, reason: &str) -> Result<(), String> {
        let from = self.record.state;
        if !Self::validate_transition(from, to) {
            return Err(format!(
                "invalid transition {} → {} (reason: {reason})",
                from.label(),
                to.label()
            ));
        }
        if to == TaskState::Completed && !self.record.verification.has_evidence() {
            return Err("cannot reach COMPLETED without verification evidence".into());
        }
        if to == TaskState::Completed && self.record.verification.concluded_by != Some(LlmRole::Reviewer) {
            return Err("only LLM 3 (Reviewer) may conclude task completion".into());
        }

        let rec = TransitionRecord {
            from_state: from,
            to_state: to,
            active_role: role,
            reason: reason.to_string(),
            timestamp: now(),
            operation_id: self.record.active_operation_id.clone(),
        };
        self.record.transitions.push(rec);
        if self.record.transitions.len() > 200 {
            let excess = self.record.transitions.len() - 200;
            self.record.transitions.drain(0..excess);
        }

        self.record.previous_state = Some(from);
        self.record.state = to;
        self.record.active_role = role;
        self.record.updated_at = now();

        if self.record.started_at.is_none() && to != TaskState::Created {
            self.record.started_at = Some(self.record.updated_at.clone());
        }
        if to.is_terminal() {
            self.record.completed_at = Some(self.record.updated_at.clone());
        }

        match to {
            TaskState::Executing => self.record.attempt_count += 1,
            TaskState::Repairing => self.record.repair_attempt_count += 1,
            TaskState::Replanning => self.record.replan_count += 1,
            TaskState::Verifying => self.record.verification_attempt_count += 1,
            _ => {}
        }

        if self.record.repair_attempt_count > self.doom.max_repair_attempts && to == TaskState::Repairing {
            return Err(format!(
                "repair limit exceeded ({} > {})",
                self.record.repair_attempt_count, self.doom.max_repair_attempts
            ));
        }
        if self.record.replan_count > self.doom.max_replans && to == TaskState::Replanning {
            return Err(format!(
                "replan limit exceeded ({} > {})",
                self.record.replan_count, self.doom.max_replans
            ));
        }

        Ok(())
    }

    pub fn set_activity(&mut self, activity: &str) {
        self.record.current_activity = activity.to_string();
        self.record.updated_at = now();
    }

    pub fn set_plan(&mut self, plan: &str, total_steps: Option<u32>) {
        self.record.current_plan = Some(plan.to_string());
        self.record.total_plan_steps = total_steps;
        self.record.current_plan_step = Some(1);
        self.record.updated_at = now();
    }

    pub fn advance_step(&mut self, step: u32, step_desc: &str) {
        self.record.current_plan_step = Some(step);
        if !step_desc.is_empty() {
            self.record.completed_steps.push(step_desc.to_string());
        }
        self.record.updated_at = now();
    }

    pub fn set_tool(&mut self, tool: &str, operation: &str, op_id: Option<&str>) {
        self.record.active_tool = Some(tool.to_string());
        self.record.active_operation = Some(operation.to_string());
        self.record.active_operation_id = op_id.map(|s| s.to_string());
        self.record.updated_at = now();
    }

    pub fn clear_tool(&mut self, result_ref: Option<&str>) {
        self.record.active_tool = None;
        self.record.active_operation = None;
        self.record.active_operation_id = None;
        if let Some(r) = result_ref {
            self.record.last_tool_result = Some(r.to_string());
        }
        self.record.updated_at = now();
    }

    pub fn record_error(&mut self, error: &str, category: &str) {
        self.record.last_error = Some(error.to_string());
        self.record.failure_category = Some(category.to_string());
        self.record.updated_at = now();
        let fp = fingerprint(error);
        self.doom.record_failure(&fp);
    }

    pub fn record_strategy(&mut self, strategy: &str) {
        let fp = fingerprint(strategy);
        self.doom.record_strategy(&fp);
    }

    pub fn record_changed_files(&mut self, files: &[String]) {
        let mut sorted = files.to_vec();
        sorted.sort();
        let fp = fingerprint(&sorted.join(","));
        self.doom.record_changed_files(&fp);
    }

    pub fn add_decision(&mut self, decision: &str) {
        self.record.important_decisions.push(decision.to_string());
        self.record.updated_at = now();
    }

    pub fn add_relevant_file(&mut self, path: &str) {
        if !self.record.relevant_files.iter().any(|f| f == path) {
            self.record.relevant_files.push(path.to_string());
        }
        self.record.updated_at = now();
    }

    pub fn set_next_action(&mut self, action: &str) {
        self.record.next_action = Some(action.to_string());
        self.record.updated_at = now();
    }

    pub fn set_checkpoint_ref(&mut self, reference: &str) {
        self.record.checkpoint_reference = Some(reference.to_string());
        self.record.updated_at = now();
    }

    pub fn add_verification_evidence(&mut self, kind: &str, status: &str, detail: &str, output_ref: Option<&str>) {
        self.record.verification.items.push(EvidenceItem {
            kind: kind.to_string(),
            status: status.to_string(),
            detail: detail.to_string(),
            output_ref: output_ref.map(|s| s.to_string()),
        });
        self.record.updated_at = now();
    }

    pub fn conclude_verification(&mut self, pass: bool) {
        self.record.verification.overall_pass = pass;
        self.record.verification.concluded_by = Some(LlmRole::Reviewer);
        self.record.verification.timestamp = Some(now());
        self.record.updated_at = now();
    }

    pub fn doom_detected(&self) -> bool {
        self.doom.doom_detected
    }

    pub fn doom_reason(&self) -> Option<&str> {
        self.doom.doom_reason.as_deref()
    }

    pub fn repair_limit_reached(&self) -> bool {
        self.record.repair_attempt_count >= self.doom.max_repair_attempts
    }

    pub fn replan_limit_reached(&self) -> bool {
        self.record.replan_count >= self.doom.max_replans
    }

    pub fn build_handoff(&self, from: LlmRole, to: LlmRole, required_action: &str) -> Handoff {
        Handoff {
            task_id: self.record.task_id.clone(),
            session_id: self.record.session_id.clone(),
            from_role: from,
            to_role: to,
            objective: self.record.current_activity.clone(),
            current_state: self.record.state,
            plan_summary: self.record.current_plan.clone().map(|p| p.chars().take(500).collect()),
            review_status: if self.record.verification.overall_pass {
                Some("pass".into())
            } else if self.record.verification.has_evidence() {
                Some("fail".into())
            } else {
                None
            },
            findings: self.record.important_decisions.clone(),
            relevant_files: self.record.relevant_files.clone(),
            important_decisions: self.record.important_decisions.clone(),
            tool_result_refs: self
                .record
                .last_tool_result
                .as_ref()
                .map(|r| vec![r.chars().take(200).collect()])
                .unwrap_or_default(),
            required_action: required_action.to_string(),
            timestamp: now(),
        }
    }

    pub fn serialize(&self) -> String {
        serde_json::to_string(&SerializableState {
            record: self.record.clone(),
            doom: self.doom.clone(),
        })
        .unwrap_or_default()
    }

    pub fn deserialize(json: &str) -> Option<Self> {
        let s: SerializableState = serde_json::from_str(json).ok()?;
        Some(Self {
            record: s.record,
            doom: s.doom,
            resume_state: None,
        })
    }

    pub fn mark_for_resume(&mut self) {
        self.resume_state = Some(self.record.state);
    }

    pub fn recovery_state(&self) -> TaskState {
        match self.record.state {
            s if s.is_terminal() => s,
            TaskState::Executing | TaskState::Repairing => TaskState::Reviewing,
            TaskState::Understanding => TaskState::Understanding,
            TaskState::Planning | TaskState::Replanning => TaskState::Planning,
            TaskState::PlanReady => TaskState::PlanReady,
            TaskState::Reviewing | TaskState::Verifying => TaskState::Reviewing,
            s if s.is_waiting() => s,
            _ => TaskState::Reviewing,
        }
    }

    pub fn render_panel(&self) -> String {
        let r = &self.record;
        let mut s = String::new();
        s.push_str("╭─ AETHER TASK STATE ─────────────────────────────────────\n");
        s.push_str(&format!("│ State: {}\n", r.state.label()));
        s.push_str(&format!("│ Active: {}\n", r.active_role.label()));
        if !r.current_activity.is_empty() {
            let act: String = r.current_activity.chars().take(60).collect();
            s.push_str(&format!("│ Activity: {act}\n"));
        }
        if let (Some(step), Some(total)) = (r.current_plan_step, r.total_plan_steps) {
            s.push_str(&format!("│ Plan: {step} / {total}\n"));
        }
        if let Some(tool) = &r.active_tool {
            s.push_str(&format!("│ Tool: {tool}\n"));
        }
        if let Some(err) = &r.last_error {
            let e: String = err.chars().take(60).collect();
            s.push_str(&format!("│ Error: {e}\n"));
        }
        if let Some(next) = &r.next_action {
            let n: String = next.chars().take(60).collect();
            s.push_str(&format!("│ Next: {n}\n"));
        }
        s.push_str(&format!(
            "│ Attempts: {} | Repairs: {} | Replans: {} | Verifications: {}\n",
            r.attempt_count, r.repair_attempt_count, r.replan_count, r.verification_attempt_count
        ));
        s.push_str("╰─────────────────────────────────────────────────────────\n");
        s
    }
}

#[derive(Serialize, Deserialize)]
struct SerializableState {
    record: TaskRecord,
    doom: DoomLoopTracker,
}

fn fingerprint(s: &str) -> String {
    let lower = s.to_lowercase();
    let normalized: String = lower
        .chars()
        .map(|c| if c.is_ascii_digit() { '0' } else { c })
        .collect();
    let mut hash: u64 = 5381;
    for b in normalized.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(b as u64);
    }
    format!("{hash:016x}")
}

// ---------------------------------------------------------------------------
// Event types for IPC
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskEventKind {
    TaskCreated { task_id: String, session_id: String },
    TaskStateChanged {
        task_id: String,
        session_id: String,
        from_state: String,
        to_state: String,
        active_role: String,
        activity: String,
        reason: String,
    },
    LlmActivated { task_id: String, role: String },
    LlmHandoff { task_id: String, from_role: String, to_role: String },
    ToolStarted { task_id: String, tool: String, operation: String },
    ToolCompleted { task_id: String, tool: String },
    ToolFailed { task_id: String, tool: String, error: String },
    FileModified { task_id: String, path: String },
    FileCreated { task_id: String, path: String },
    FileDeleted { task_id: String, path: String },
    ContextWarning {
        task_id: String,
        estimated_tokens: u32,
        context_window: u32,
        health: String,
    },
    CompactionStarted { task_id: String, trigger: String },
    CompactionCompleted { task_id: String, tokens_before: u32, tokens_after: u32 },
    CompactionFailed { task_id: String, error: String },
    VerificationStarted { task_id: String, check: String },
    VerificationCompleted { task_id: String, check: String, status: String, detail: String },
    TaskCompleted { task_id: String, session_id: String },
    TaskFailed { task_id: String, session_id: String, reason: String },
    TaskCancelled { task_id: String, session_id: String },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sm() -> TaskStateMachine {
        TaskStateMachine::new("task-1", "session-1")
    }

    #[test]
    fn normal_lifecycle() {
        let mut m = sm();
        assert_eq!(m.state(), TaskState::Created);

        m.transition(TaskState::Understanding, LlmRole::Reviewer, "start understanding").unwrap();
        assert_eq!(m.state(), TaskState::Understanding);
        assert_eq!(m.active_role(), LlmRole::Reviewer);

        m.transition(TaskState::Planning, LlmRole::Planner, "plan").unwrap();
        m.transition(TaskState::PlanReady, LlmRole::Planner, "plan ready").unwrap();
        m.transition(TaskState::Executing, LlmRole::Executor, "execute").unwrap();
        m.transition(TaskState::Reviewing, LlmRole::Reviewer, "review").unwrap();
        m.transition(TaskState::Verifying, LlmRole::Reviewer, "verify").unwrap();

        m.add_verification_evidence("tests", "pass", "27 passed / 0 failed", None);
        m.conclude_verification(true);
        m.transition(TaskState::Completed, LlmRole::Reviewer, "verified").unwrap();
        assert_eq!(m.state(), TaskState::Completed);
        assert!(m.record.completed_at.is_some());
    }

    #[test]
    fn failed_verification_replan_repair_cycle() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        m.transition(TaskState::Planning, LlmRole::Planner, "p").unwrap();
        m.transition(TaskState::PlanReady, LlmRole::Planner, "pr").unwrap();
        m.transition(TaskState::Executing, LlmRole::Executor, "e").unwrap();
        m.transition(TaskState::Reviewing, LlmRole::Reviewer, "r").unwrap();
        m.transition(TaskState::Verifying, LlmRole::Reviewer, "v").unwrap();

        m.add_verification_evidence("tests", "fail", "3 failed", None);
        m.conclude_verification(false);
        m.transition(TaskState::Replanning, LlmRole::Planner, "verification failed").unwrap();
        m.transition(TaskState::Repairing, LlmRole::Executor, "repair").unwrap();
        m.transition(TaskState::Reviewing, LlmRole::Reviewer, "re-review").unwrap();
        m.transition(TaskState::Verifying, LlmRole::Reviewer, "re-verify").unwrap();

        m.add_verification_evidence("tests", "pass", "all pass", None);
        m.conclude_verification(true);
        m.transition(TaskState::Completed, LlmRole::Reviewer, "done").unwrap();
        assert_eq!(m.record.repair_attempt_count, 1);
        assert_eq!(m.record.replan_count, 1);
    }

    #[test]
    fn invalid_transitions_rejected() {
        let mut m = sm();
        assert!(m.transition(TaskState::Executing, LlmRole::Executor, "skip").is_err());
        assert!(m.transition(TaskState::Completed, LlmRole::Executor, "skip").is_err());
        assert!(m.transition(TaskState::Verifying, LlmRole::Reviewer, "skip").is_err());

        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        assert!(m.transition(TaskState::Executing, LlmRole::Executor, "skip planning").is_err());
        assert!(m.transition(TaskState::Completed, LlmRole::Executor, "skip").is_err());
    }

    #[test]
    fn executor_cannot_complete() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        m.transition(TaskState::Planning, LlmRole::Planner, "p").unwrap();
        m.transition(TaskState::PlanReady, LlmRole::Planner, "pr").unwrap();
        m.transition(TaskState::Executing, LlmRole::Executor, "e").unwrap();
        m.transition(TaskState::Reviewing, LlmRole::Reviewer, "r").unwrap();
        m.transition(TaskState::Verifying, LlmRole::Reviewer, "v").unwrap();
        m.add_verification_evidence("tests", "pass", "ok", None);
        let result = m.transition(TaskState::Completed, LlmRole::Executor, "executor tries to complete");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("LLM 3"));
    }

    #[test]
    fn completion_requires_evidence() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        m.transition(TaskState::Planning, LlmRole::Planner, "p").unwrap();
        m.transition(TaskState::PlanReady, LlmRole::Planner, "pr").unwrap();
        m.transition(TaskState::Executing, LlmRole::Executor, "e").unwrap();
        m.transition(TaskState::Reviewing, LlmRole::Reviewer, "r").unwrap();
        m.transition(TaskState::Verifying, LlmRole::Reviewer, "v").unwrap();
        let result = m.transition(TaskState::Completed, LlmRole::Reviewer, "no evidence");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("evidence"));
    }

    #[test]
    fn terminal_states_are_final() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        m.transition(TaskState::Planning, LlmRole::Planner, "p").unwrap();
        m.transition(TaskState::PlanReady, LlmRole::Planner, "pr").unwrap();
        m.transition(TaskState::Executing, LlmRole::Executor, "e").unwrap();
        m.transition(TaskState::Cancelled, LlmRole::System, "user cancel").unwrap();
        assert!(m.transition(TaskState::Executing, LlmRole::Executor, "resume").is_err());
        assert!(m.transition(TaskState::Reviewing, LlmRole::Reviewer, "resume").is_err());
    }

    #[test]
    fn cancellation_from_active_states() {
        for start in [TaskState::Executing, TaskState::Planning, TaskState::Reviewing, TaskState::Verifying, TaskState::Repairing] {
            let mut m = sm();
            m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
            m.transition(TaskState::Planning, LlmRole::Planner, "p").unwrap();
            m.transition(TaskState::PlanReady, LlmRole::Planner, "pr").unwrap();
            if start == TaskState::Planning {
                m.transition(TaskState::Cancelled, LlmRole::System, "cancel").unwrap();
                assert_eq!(m.state(), TaskState::Cancelled);
                continue;
            }
            m.transition(TaskState::Executing, LlmRole::Executor, "e").unwrap();
            if start == TaskState::Executing {
                m.transition(TaskState::Cancelled, LlmRole::System, "cancel").unwrap();
                assert_eq!(m.state(), TaskState::Cancelled);
                continue;
            }
            m.transition(TaskState::Reviewing, LlmRole::Reviewer, "r").unwrap();
            if start == TaskState::Reviewing {
                m.transition(TaskState::Cancelled, LlmRole::System, "cancel").unwrap();
                assert_eq!(m.state(), TaskState::Cancelled);
                continue;
            }
            m.transition(TaskState::Verifying, LlmRole::Reviewer, "v").unwrap();
            if start == TaskState::Verifying {
                m.transition(TaskState::Cancelled, LlmRole::System, "cancel").unwrap();
                assert_eq!(m.state(), TaskState::Cancelled);
                continue;
            }
            m.add_verification_evidence("t", "fail", "f", None);
            m.conclude_verification(false);
            m.transition(TaskState::Replanning, LlmRole::Planner, "rp").unwrap();
            m.transition(TaskState::Repairing, LlmRole::Executor, "rep").unwrap();
            m.transition(TaskState::Cancelled, LlmRole::System, "cancel").unwrap();
            assert_eq!(m.state(), TaskState::Cancelled);
        }
    }

    #[test]
    fn repair_limit_enforced() {
        let mut m = TaskStateMachine::new("t", "s").with_limits(2, 5);
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        m.transition(TaskState::Planning, LlmRole::Planner, "p").unwrap();
        m.transition(TaskState::PlanReady, LlmRole::Planner, "pr").unwrap();
        m.transition(TaskState::Executing, LlmRole::Executor, "e").unwrap();
        m.transition(TaskState::Reviewing, LlmRole::Reviewer, "r").unwrap();
        m.transition(TaskState::Verifying, LlmRole::Reviewer, "v").unwrap();
        m.add_verification_evidence("t", "fail", "f", None);
        m.conclude_verification(false);

        m.transition(TaskState::Replanning, LlmRole::Planner, "rp1").unwrap();
        m.transition(TaskState::Repairing, LlmRole::Executor, "rep1").unwrap();
        m.transition(TaskState::Reviewing, LlmRole::Reviewer, "r2").unwrap();
        m.transition(TaskState::Verifying, LlmRole::Reviewer, "v2").unwrap();
        m.conclude_verification(false);

        m.transition(TaskState::Replanning, LlmRole::Planner, "rp2").unwrap();
        m.transition(TaskState::Repairing, LlmRole::Executor, "rep2").unwrap();
        m.transition(TaskState::Reviewing, LlmRole::Reviewer, "r3").unwrap();
        m.transition(TaskState::Verifying, LlmRole::Reviewer, "v3").unwrap();
        m.conclude_verification(false);

        m.transition(TaskState::Replanning, LlmRole::Planner, "rp3").unwrap();
        let result = m.transition(TaskState::Repairing, LlmRole::Executor, "rep3 exceeds limit");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("repair limit"));
    }

    #[test]
    fn doom_loop_detection() {
        let mut m = sm();
        m.record_error("compile error at src/main.rs:42", "build");
        m.record_error("compile error at src/main.rs:42", "build");
        m.record_error("compile error at src/main.rs:42", "build");
        assert!(m.doom_detected());
        assert!(m.doom_reason().unwrap().contains("failure fingerprint"));
    }

    #[test]
    fn different_failures_no_doom() {
        let mut m = sm();
        m.record_error("error A at line 1", "build");
        m.record_error("error B at line 2", "test");
        m.record_error("error C at line 3", "lint");
        assert!(!m.doom_detected());
    }

    #[test]
    fn session_isolation() {
        let mut a = TaskStateMachine::new("task-a", "session-a");
        let mut b = TaskStateMachine::new("task-b", "session-b");
        a.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        a.transition(TaskState::Planning, LlmRole::Planner, "p").unwrap();
        assert_eq!(a.state(), TaskState::Planning);
        assert_eq!(b.state(), TaskState::Created);
        assert_eq!(a.record.session_id, "session-a");
        assert_eq!(b.record.session_id, "session-b");
    }

    #[test]
    fn serialization_roundtrip() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        m.transition(TaskState::Planning, LlmRole::Planner, "p").unwrap();
        m.set_activity("planning auth");
        m.set_plan("step 1\nstep 2", Some(2));
        m.add_relevant_file("src/auth.ts");
        m.add_decision("use JWT");

        let json = m.serialize();
        let restored = TaskStateMachine::deserialize(&json).unwrap();
        assert_eq!(restored.state(), TaskState::Planning);
        assert_eq!(restored.record.current_activity, "planning auth");
        assert_eq!(restored.record.relevant_files, vec!["src/auth.ts"]);
        assert_eq!(restored.record.important_decisions, vec!["use JWT"]);
        assert_eq!(restored.record.transitions.len(), 2);
    }

    #[test]
    fn model_assignments_never_change() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        m.transition(TaskState::Planning, LlmRole::Planner, "p").unwrap();
        m.transition(TaskState::PlanReady, LlmRole::Planner, "pr").unwrap();
        m.transition(TaskState::Executing, LlmRole::Executor, "e").unwrap();
        m.transition(TaskState::Reviewing, LlmRole::Reviewer, "r").unwrap();
        m.transition(TaskState::Verifying, LlmRole::Reviewer, "v").unwrap();
        m.add_verification_evidence("t", "pass", "ok", None);
        m.conclude_verification(true);
        m.transition(TaskState::Completed, LlmRole::Reviewer, "done").unwrap();
        assert_eq!(m.active_role(), LlmRole::Reviewer);
    }

    #[test]
    fn waiting_states_and_resume() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        m.transition(TaskState::WaitingUser, LlmRole::System, "need clarification").unwrap();
        assert!(m.state().is_waiting());
        m.transition(TaskState::Planning, LlmRole::Planner, "user responded").unwrap();
        assert_eq!(m.state(), TaskState::Planning);
    }

    #[test]
    fn waiting_tool_and_network() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        m.transition(TaskState::Planning, LlmRole::Planner, "p").unwrap();
        m.transition(TaskState::PlanReady, LlmRole::Planner, "pr").unwrap();
        m.transition(TaskState::Executing, LlmRole::Executor, "e").unwrap();
        m.transition(TaskState::WaitingTool, LlmRole::System, "tool pending").unwrap();
        m.transition(TaskState::Executing, LlmRole::Executor, "tool done").unwrap();
        m.transition(TaskState::WaitingNetwork, LlmRole::System, "provider timeout").unwrap();
        m.transition(TaskState::Executing, LlmRole::Executor, "network back").unwrap();
        assert_eq!(m.state(), TaskState::Executing);
    }

    #[test]
    fn crash_recovery_from_executing() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        m.transition(TaskState::Planning, LlmRole::Planner, "p").unwrap();
        m.transition(TaskState::PlanReady, LlmRole::Planner, "pr").unwrap();
        m.transition(TaskState::Executing, LlmRole::Executor, "e").unwrap();
        let json = m.serialize();
        let restored = TaskStateMachine::deserialize(&json).unwrap();
        assert_eq!(restored.recovery_state(), TaskState::Reviewing);
    }

    #[test]
    fn crash_recovery_from_terminal() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        m.transition(TaskState::Cancelled, LlmRole::System, "cancel").unwrap();
        let json = m.serialize();
        let restored = TaskStateMachine::deserialize(&json).unwrap();
        assert_eq!(restored.recovery_state(), TaskState::Cancelled);
    }

    #[test]
    fn transition_history_recorded() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "start").unwrap();
        m.transition(TaskState::Planning, LlmRole::Planner, "plan").unwrap();
        assert_eq!(m.record.transitions.len(), 2);
        assert_eq!(m.record.transitions[0].from_state, TaskState::Created);
        assert_eq!(m.record.transitions[0].to_state, TaskState::Understanding);
        assert_eq!(m.record.transitions[0].active_role, LlmRole::Reviewer);
        assert_eq!(m.record.transitions[1].from_state, TaskState::Understanding);
        assert_eq!(m.record.transitions[1].to_state, TaskState::Planning);
    }

    #[test]
    fn handoff_structure() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        m.set_activity("implement auth");
        m.add_relevant_file("src/auth.ts");
        m.add_decision("use JWT");
        let h = m.build_handoff(LlmRole::Reviewer, LlmRole::Planner, "create plan");
        assert_eq!(h.from_role, LlmRole::Reviewer);
        assert_eq!(h.to_role, LlmRole::Planner);
        assert_eq!(h.task_id, "task-1");
        assert_eq!(h.session_id, "session-1");
        assert_eq!(h.required_action, "create plan");
        assert_eq!(h.relevant_files, vec!["src/auth.ts"]);
    }

    #[test]
    fn duplicate_transition_rejected() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        let result = m.transition(TaskState::Understanding, LlmRole::Reviewer, "duplicate");
        assert!(result.is_err());
    }

    #[test]
    fn plan_ready_to_replanning() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        m.transition(TaskState::Planning, LlmRole::Planner, "p").unwrap();
        m.transition(TaskState::PlanReady, LlmRole::Planner, "pr").unwrap();
        m.transition(TaskState::Replanning, LlmRole::Planner, "plan review failed").unwrap();
        m.transition(TaskState::PlanReady, LlmRole::Planner, "revised plan").unwrap();
        m.transition(TaskState::Executing, LlmRole::Executor, "execute").unwrap();
        assert_eq!(m.state(), TaskState::Executing);
    }

    #[test]
    fn blocked_state() {
        let mut m = sm();
        m.transition(TaskState::Understanding, LlmRole::Reviewer, "u").unwrap();
        m.transition(TaskState::Planning, LlmRole::Planner, "p").unwrap();
        m.transition(TaskState::PlanReady, LlmRole::Planner, "pr").unwrap();
        m.transition(TaskState::Executing, LlmRole::Executor, "e").unwrap();
        m.transition(TaskState::Blocked, LlmRole::System, "dependency missing").unwrap();
        assert_eq!(m.state(), TaskState::Blocked);
        m.transition(TaskState::WaitingUser, LlmRole::System, "ask user").unwrap();
        assert_eq!(m.state(), TaskState::WaitingUser);
    }
}
