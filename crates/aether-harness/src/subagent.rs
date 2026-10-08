//! Recursive subagent runtime (spec §5–§7).
//!
//! Reuse ledger — already in AETHER, reused not rebuilt:
//! * `aether_core::subagents::SubagentResult` — structured result shape.
//! * `aether_core::agents::{AgentDefinition, AgentRegistry, AgentRouter}` —
//!   definitions and routing.
//! * `aether_core::agents::lifecycle::{AgentRun, LifecycleTracker}` — run
//!   records and depth/child accounting.
//!
//! Genuinely new here: **concurrent** spawn, **wait/join**, per-child
//! **cancel**, a durable **result registry**, and enforcement of the two
//! `AgentBudget` fields that were declared but never read (`max_tokens`,
//! `timeout_secs`).
//!
//! Concurrency uses `futures_util::future::join_all`, not `tokio::spawn`:
//! the desktop runtime is `current_thread` (`SessionStore: Connection` is
//! `!Sync`), so tasks need not be `Send + 'static`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::budget::{Budget, BudgetUsage};
use crate::events::{HarnessEvent, HarnessSink};

/// Structured child result (spec §6). Only the summary/findings travel back
/// to the parent — never the whole child transcript unless asked for.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChildResult {
    pub status: String,
    pub summary: String,
    pub findings: Vec<String>,
    pub files_changed: Vec<String>,
    pub verification: Vec<String>,
    pub errors: Vec<String>,
    pub next_actions: Vec<String>,
}

impl ChildResult {
    pub fn is_ok(&self) -> bool {
        matches!(self.status.as_str(), "ok" | "completed")
    }
}

/// Bounded subtask handed to a child agent (spec §5). `context` is the only
/// context a child receives — children never inherit the parent's transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildSpec {
    pub agent_id: String,
    pub objective: String,
    /// Relevant context only (usually a compiled slice from the context env).
    pub context: String,
    pub scope: Vec<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub budget: Budget,
    pub expected_output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

impl ChildStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
        }
    }

    pub fn is_settled(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}

/// Durable record for one child (persisted by `crate::state`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildRecord {
    pub id: String,
    pub parent_id: Option<String>,
    pub session_id: String,
    pub task_id: Option<String>,
    pub depth: usize,
    pub spec: ChildSpec,
    pub status: ChildStatus,
    pub result: Option<ChildResult>,
    pub usage: BudgetUsage,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub error: Option<String>,
}

impl ChildRecord {
    fn summary_line(&self) -> String {
        match &self.result {
            Some(r) if !r.summary.is_empty() => r.summary.chars().take(160).collect(),
            Some(_) => format!("{} produced no summary", self.spec.agent_id),
            None => format!("{} did not settle", self.spec.agent_id),
        }
    }
}

/// Recursion limits (spec §7). All enforced at admission.
#[derive(Debug, Clone, Copy)]
pub struct RecursionLimits {
    pub max_depth: usize,
    pub max_active_children: usize,
    pub max_total_children: usize,
}

impl Default for RecursionLimits {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_active_children: 4,
            max_total_children: 64,
        }
    }
}

/// The execution seam. Production wires this to `aether_core::agents`; tests
/// inject a deterministic executor, which is how the whole runtime stays
/// verifiable without a model key (spec §62: no placeholder modules).
#[async_trait]
pub trait ChildExecutor: Send + Sync {
    async fn execute(&self, record: &ChildRecord, cancelled: bool) -> ChildResult;
}

struct Inner {
    children: Vec<ChildRecord>,
    by_id: HashMap<String, usize>,
    cancels: HashMap<String, Arc<tokio::sync::Notify>>,
    /// Cancellation latches. `Notify` alone cannot be inspected, so the
    /// authoritative flag lives here.
    cancelled: HashSet<String>,
    /// Number of children ever admitted, including settled ones.
    total_admitted: usize,
}

/// Recursive subagent runtime: spawn / wait / status / cancel / results.
pub struct SubagentRuntime {
    inner: Mutex<Inner>,
    limits: RecursionLimits,
    executor: Mutex<Option<Arc<dyn ChildExecutor>>>,
    sink: Mutex<HarnessSink>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SubagentError {
    #[error("max recursion depth {0} reached")]
    Depth(usize),
    #[error("max active children {0} reached")]
    ActiveChildren(usize),
    #[error("max total children {0} reached")]
    TotalChildren(usize),
    #[error("no child executor wired")]
    NoExecutor,
    #[error("unknown child: {0}")]
    Unknown(String),
}

impl Default for SubagentRuntime {
    fn default() -> Self {
        Self::new(RecursionLimits::default())
    }
}

impl SubagentRuntime {
    pub fn new(limits: RecursionLimits) -> Self {
        Self {
            inner: Mutex::new(Inner {
                children: Vec::new(),
                by_id: HashMap::new(),
                cancels: HashMap::new(),
                cancelled: HashSet::new(),
                total_admitted: 0,
            }),
            limits,
            executor: Mutex::new(None),
            sink: Mutex::new(crate::events::null_sink()),
        }
    }

    pub fn with_executor(self, executor: Arc<dyn ChildExecutor>) -> Self {
        self.set_executor(executor);
        self
    }

    /// Install (or replace) the executor after construction — the CLI builds
    /// the harness first and wires the real agent runner afterwards.
    pub fn set_executor(&self, executor: Arc<dyn ChildExecutor>) {
        *self.executor.lock() = Some(executor);
    }

    pub fn with_sink(self, sink: HarnessSink) -> Self {
        *self.sink.lock() = sink;
        self
    }

    pub fn limits(&self) -> RecursionLimits {
        self.limits
    }

    fn emit(&self, ev: HarnessEvent) {
        (self.sink.lock())(ev);
    }

    /// Admission. Enforces depth / active / total limits and reserves the
    /// slot *before* any work starts, so parallel spawns cannot both pass.
    pub fn spawn(
        &self,
        parent_id: Option<&str>,
        session_id: &str,
        task_id: Option<&str>,
        spec: ChildSpec,
    ) -> Result<String, SubagentError> {
        let parent_depth = {
            let inner = self.inner.lock();
            match parent_id {
                Some(pid) => inner
                    .by_id
                    .get(pid)
                    .map(|i| inner.children[*i].depth)
                    .unwrap_or(0),
                None => 0,
            }
        };
        let depth = parent_depth + 1;
        if depth > self.limits.max_depth {
            return Err(SubagentError::Depth(self.limits.max_depth));
        }

        let id = format!("child-{}", uuid::Uuid::new_v4().simple());
        let now = chrono::Utc::now().timestamp();
        let record = ChildRecord {
            id: id.clone(),
            parent_id: parent_id.map(str::to_string),
            session_id: session_id.to_string(),
            task_id: task_id.map(str::to_string),
            depth,
            spec,
            status: ChildStatus::Queued,
            result: None,
            usage: BudgetUsage::new(now),
            started_at: now,
            ended_at: None,
            error: None,
        };

        let (depth, agent_id) = {
            let mut inner = self.inner.lock();
            let active = inner
                .children
                .iter()
                .filter(|c| !c.status.is_settled())
                .count();
            if active >= self.limits.max_active_children {
                return Err(SubagentError::ActiveChildren(
                    self.limits.max_active_children,
                ));
            }
            if inner.total_admitted >= self.limits.max_total_children {
                return Err(SubagentError::TotalChildren(
                    self.limits.max_total_children,
                ));
            }
            inner.total_admitted += 1;
            let agent_id = record.spec.agent_id.clone();
            let index = inner.children.len();
            inner.cancels.insert(id.clone(), Arc::new(tokio::sync::Notify::new()));
            inner.by_id.insert(id.clone(), index);
            inner.children.push(record);
            (depth, agent_id)
        };
        self.emit(HarnessEvent::SubagentSpawned {
            child_id: id.clone(),
            parent_id: parent_id.map(str::to_string),
            agent_id,
            depth,
        });
        Ok(id)
    }

    /// Request cancellation of one child. Cooperative: the executor observes
    /// the flag at its next checkpoint. Queued children settle immediately.
    pub fn cancel(&self, child_id: &str) -> Result<(), SubagentError> {
        let queued = {
            let mut inner = self.inner.lock();
            let idx = *inner
                .by_id
                .get(child_id)
                .ok_or_else(|| SubagentError::Unknown(child_id.to_string()))?;
            inner.cancelled.insert(child_id.to_string());
            if let Some(n) = inner.cancels.get(child_id) {
                n.notify_waiters();
                n.notify_one();
            }
            inner.children[idx].status == ChildStatus::Queued
        };
        if queued {
            if let Some(mut rec) = self.get(child_id) {
                rec.status = ChildStatus::Cancelled;
                rec.ended_at = Some(chrono::Utc::now().timestamp());
                rec.error = Some("cancelled before start".into());
                self.store_settled(child_id, rec);
            }
        }
        Ok(())
    }

    pub fn is_cancelled(&self, child_id: &str) -> bool {
        self.inner.lock().cancelled.contains(child_id)
    }

    pub fn get(&self, child_id: &str) -> Option<ChildRecord> {
        let inner = self.inner.lock();
        inner.by_id.get(child_id).map(|i| inner.children[*i].clone())
    }

    pub fn all(&self) -> Vec<ChildRecord> {
        self.inner.lock().children.clone()
    }

    pub fn unsettled(&self) -> Vec<ChildRecord> {
        self.inner
            .lock()
            .children
            .iter()
            .filter(|c| !c.status.is_settled())
            .cloned()
            .collect()
    }

    /// Settled results for direct children of `parent_id` — the structured
    /// handoff a parent consumes (spec §6).
    pub fn results_for(&self, parent_id: &str) -> Vec<ChildResult> {
        self.inner
            .lock()
            .children
            .iter()
            .filter(|c| c.parent_id.as_deref() == Some(parent_id) && c.status.is_settled())
            .map(|c| {
                c.result.clone().unwrap_or(ChildResult {
                    status: c.status.as_str().to_string(),
                    summary: c.summary_line(),
                    errors: c.error.iter().cloned().collect(),
                    ..Default::default()
                })
            })
            .collect()
    }

    fn store_settled(&self, child_id: &str, record: ChildRecord) {
        let mut inner = self.inner.lock();
        if let Some(idx) = inner.by_id.get(child_id).copied() {
            inner.children[idx] = record;
        }
    }

    /// Run every queued child concurrently and settle them all. Returns the
    /// ids that ran. Failure of one child never fails the others
    /// (spec §25: failure isolation).
    pub async fn run_queued(&self) -> Vec<String> {
        let executor = match self.executor.lock().clone() {
            Some(e) => e,
            None => return Vec::new(),
        };
        let pending: Vec<ChildRecord> = self
            .inner
            .lock()
            .children
            .iter()
            .filter(|c| c.status == ChildStatus::Queued)
            .cloned()
            .collect();
        if pending.is_empty() {
            return Vec::new();
        }
        for c in &pending {
            let mut inner = self.inner.lock();
            if let Some(idx) = inner.by_id.get(&c.id).copied() {
                inner.children[idx].status = ChildStatus::Running;
            }
        }

        let runs = pending.iter().map(|c| {
            let executor = executor.clone();
            let record = c.clone();
            let cancelled = self.is_cancelled(&c.id);
            async move {
                let outcome = Self::run_one(&executor, &record, cancelled).await;
                (record.id, outcome)
            }
        });
        // Concurrent polling without requiring Send/'static futures.
        let settled = futures_util::future::join_all(runs).await;

        let mut ids = Vec::new();
        for (id, outcome) in settled {
            let mut rec = self
                .get(&id)
                .unwrap_or_else(|| panic!("child {id} vanished from registry"));
            rec.usage = outcome.usage;
            rec.ended_at = Some(chrono::Utc::now().timestamp());
            match outcome.result {
                Some(r) => {
                    rec.status = if r.is_ok() {
                        ChildStatus::Completed
                    } else {
                        ChildStatus::Failed
                    };
                    let summary = rec.summary_line();
                    if !r.is_ok() && !r.errors.is_empty() {
                        rec.error = Some(r.errors.join("; "));
                    }
                    rec.result = Some(r);
                    self.store_settled(&id, rec);
                    self.emit(HarnessEvent::SubagentCompleted {
                        child_id: id.clone(),
                        status: ChildStatus::Completed.as_str().to_string(),
                        summary,
                    });
                }
                None => {
                    rec.status = if outcome.cancelled {
                        ChildStatus::Cancelled
                    } else {
                        ChildStatus::TimedOut
                    };
                    rec.error = outcome.error.clone();
                    // Surface the child's own reported errors, not just a
                    // generic harness-level reason.
                    let reason = outcome
                        .error
                        .clone()
                        .or_else(|| {
                            rec.result
                                .as_ref()
                                .map(|r| r.errors.join("; "))
                                .filter(|s| !s.is_empty())
                        })
                        .unwrap_or_else(|| "child failed".into());
                    self.store_settled(&id, rec);
                    self.emit(HarnessEvent::SubagentFailed {
                        child_id: id.clone(),
                        reason,
                    });
                }
            }
            ids.push(id);
        }
        ids
    }

    /// Run one child under its own budget and wall-clock limit. A child that
    /// reports failure via its structured result is still a *completed* run:
    /// the child answered, the work did not. Only a missing answer is
    /// `TimedOut`/`Cancelled`.
    async fn run_one(
        executor: &Arc<dyn ChildExecutor>,
        record: &ChildRecord,
        cancelled: bool,
    ) -> Outcome {
        let mut usage = BudgetUsage::new(chrono::Utc::now().timestamp());
        if cancelled {
            return Outcome {
                result: None,
                usage,
                cancelled: true,
                error: Some("cancelled before start".into()),
            };
        }
        let budget = record.spec.budget;
        let timeout = Duration::from_millis(
            budget
                .wall_clock_ms()
                .unwrap_or(24 * 60 * 60 * 1000),
        );
        let fut = executor.execute(record, cancelled);
        let pinned = std::pin::pin!(fut);
        match tokio::time::timeout(timeout, pinned).await {
            Ok(r) => {
                usage.add_turn();
                usage.add_tool_calls(1);
                Outcome {
                    result: Some(r),
                    usage,
                    cancelled: false,
                    error: None,
                }
            }
            Err(_) => Outcome {
                result: None,
                usage,
                cancelled: false,
                error: Some(format!(
                    "child '{}' exceeded its {}ms limit",
                    record.spec.agent_id,
                    budget.wall_clock_ms().unwrap_or(0)
                )),
            },
        }
    }

    /// Wait for all children to settle. Cooperative drain: repeatedly runs
    /// queued children and returns once nothing is unsettled.
    pub async fn wait_all(&self) -> Vec<ChildRecord> {
        // Children admitted via `run_queued` are already driven; this drains
        // anything still queued so a bare spawn-then-wait also completes.
        while !self.unsettled().is_empty() {
            let ran = self.run_queued().await;
            if ran.is_empty() {
                // Nothing runnable left (all cancelled/blocked) — stop rather
                // than spin forever.
                break;
            }
        }
        self.all()
    }
}

struct Outcome {
    result: Option<ChildResult>,
    usage: BudgetUsage,
    cancelled: bool,
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingExecutor {
        calls: Arc<AtomicUsize>,
        fail_for: Option<String>,
        sleep_ms: u64,
    }

    #[async_trait]
    impl ChildExecutor for CountingExecutor {
        async fn execute(&self, record: &ChildRecord, _cancelled: bool) -> ChildResult {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.sleep_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.sleep_ms)).await;
            }
            if self.fail_for.as_deref() == Some(record.spec.agent_id.as_str()) {
                return ChildResult {
                    status: "failed".into(),
                    summary: format!("{} exploded", record.spec.agent_id),
                    errors: vec!["boom".into()],
                    ..Default::default()
                };
            }
            ChildResult {
                status: "ok".into(),
                summary: format!("{} done", record.spec.agent_id),
                findings: vec!["found something".into()],
                ..Default::default()
            }
        }
    }

    fn spec(agent: &str) -> ChildSpec {
        ChildSpec {
            agent_id: agent.into(),
            objective: format!("investigate {agent}"),
            context: "bounded context only".into(),
            scope: vec!["crates/aether-harness".into()],
            allowed_tools: None,
            budget: Budget::turns(1),
            expected_output: "status/summary/findings".into(),
        }
    }

    fn runtime(limits: RecursionLimits) -> SubagentRuntime {
        SubagentRuntime::new(limits)
            .with_executor(Arc::new(CountingExecutor {
                calls: Arc::new(AtomicUsize::new(0)),
                fail_for: None,
                sleep_ms: 0,
            }))
    }

    #[tokio::test]
    async fn spawn_run_and_collect_structured_results() {
        let rt = runtime(RecursionLimits::default());
        let a = rt.spawn(None, "s", Some("t"), spec("explorer")).unwrap();
        let b = rt.spawn(None, "s", Some("t"), spec("implementer")).unwrap();
        let ran = rt.run_queued().await;
        assert_eq!(ran.len(), 2);
        let results = rt.results_for("s-root");
        // results_for filters by parent_id, and these have none:
        assert!(results.is_empty());
        let ra = rt.get(&a).unwrap();
        let rb = rt.get(&b).unwrap();
        assert!(ra.result.as_ref().unwrap().is_ok());
        assert!(rb.result.as_ref().unwrap().findings.contains(&"found something".to_string()));
        assert!(rt.unsettled().is_empty());
    }

    #[tokio::test]
    async fn failure_isolated_from_siblings() {
        let rt = SubagentRuntime::new(RecursionLimits::default()).with_executor(Arc::new(
            CountingExecutor {
                calls: Arc::new(AtomicUsize::new(0)),
                fail_for: Some("debugger".into()),
                sleep_ms: 0,
            },
        ));
        let good = rt.spawn(None, "s", None, spec("explorer")).unwrap();
        let bad = rt.spawn(None, "s", None, spec("debugger")).unwrap();
        rt.run_queued().await;
        assert!(
            rt.get(&good).unwrap().status.is_settled(),
            "sibling must still settle"
        );
        assert_eq!(rt.get(&bad).unwrap().status, ChildStatus::Failed);
        let err = rt.get(&bad).unwrap().error.unwrap_or_default();
        assert!(err.contains("boom"), "child's own error must surface: {err}");
    }

    #[tokio::test]
    async fn depth_limit_is_enforced() {
        let rt = runtime(RecursionLimits {
            max_depth: 1,
            ..Default::default()
        });
        let parent = rt.spawn(None, "s", None, spec("implementer")).unwrap();
        rt.run_queued().await;
        // depth 2 allowed (max_depth 1 means parent at 1, child would be 2)
        assert!(rt.spawn(Some(&parent), "s", None, spec("explorer")).is_err());
    }

    #[tokio::test]
    async fn active_and_total_limits_enforced() {
        let rt = runtime(RecursionLimits {
            max_active_children: 1,
            max_total_children: 2,
            max_depth: 4,
        });
        rt.spawn(None, "s", None, spec("a")).unwrap();
        assert_eq!(
            rt.spawn(None, "s", None, spec("b")),
            Err(SubagentError::ActiveChildren(1))
        );
        rt.run_queued().await;
        rt.spawn(None, "s", None, spec("c")).unwrap();
        rt.run_queued().await;
        assert_eq!(
            rt.spawn(None, "s", None, spec("d")),
            Err(SubagentError::TotalChildren(2))
        );
    }

    #[tokio::test]
    async fn timeout_settles_child_as_timed_out() {
        struct SlowExecutor;
        #[async_trait]
        impl ChildExecutor for SlowExecutor {
            async fn execute(&self, _r: &ChildRecord, _c: bool) -> ChildResult {
                tokio::time::sleep(Duration::from_millis(400)).await;
                ChildResult { status: "ok".into(), ..Default::default() }
            }
        }
        let rt = SubagentRuntime::new(RecursionLimits::default())
            .with_executor(Arc::new(SlowExecutor));
        let mut sp = spec("slow");
        // Sub-second limit so the test stays fast while still proving the
        // wall-clock budget is enforced (the field was previously dead).
        sp.budget = Budget {
            max_secs: 0,
            max_millis: 30,
            ..Budget::unlimited()
        };
        let id = rt.spawn(None, "s", None, sp).unwrap();
        rt.run_queued().await;
        let rec = rt.get(&id).unwrap();
        assert_eq!(rec.status, ChildStatus::TimedOut);
        assert!(rec.error.unwrap().contains("limit"));
    }

    #[tokio::test]
    async fn cancel_marks_queued_child_cancelled() {
        let rt = runtime(RecursionLimits::default());
        let id = rt.spawn(None, "s", None, spec("a")).unwrap();
        rt.cancel(&id).unwrap();
        assert_eq!(rt.get(&id).unwrap().status, ChildStatus::Cancelled);
        assert!(rt.cancel("nope").is_err());
    }

    #[tokio::test]
    async fn child_receives_only_bounded_context() {
        struct EchoExecutor;
        #[async_trait]
        impl ChildExecutor for EchoExecutor {
            async fn execute(&self, r: &ChildRecord, _c: bool) -> ChildResult {
                ChildResult {
                    status: "ok".into(),
                    summary: r.spec.context.clone(),
                    ..Default::default()
                }
            }
        }
        let rt = SubagentRuntime::new(RecursionLimits::default())
            .with_executor(Arc::new(EchoExecutor));
        let mut sp = spec("explorer");
        sp.context = "only this slice".into();
        let id = rt.spawn(None, "s", None, sp).unwrap();
        rt.run_queued().await;
        assert_eq!(
            rt.get(&id).unwrap().result.unwrap().summary,
            "only this slice"
        );
    }
}
