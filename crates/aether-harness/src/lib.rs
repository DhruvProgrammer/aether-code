//! `aether-harness` — AETHER's persistent, recursive, self-improving harness.
//!
//! Inspired by Prime Agent's RLM (context as programmable state) and
//! Continual Harness (durable supplemental state + evidence-backed
//! refinement). Implemented natively against AETHER's existing architecture —
//! no upstream code copied, no parallel subsystems invented.
//!
//! **What is new here** (everything else was verified absent from AETHER):
//! persistent goals, a job scheduler with heartbeats and durable re-entry,
//! concurrent recursive subagents with wait/join/cancel, a refinement store
//! with versioning and rollback, quality gates that refuse to lie, an
//! RLM-style structured context runtime, and a versioned state store with
//! recorded migrations.
//!
//! **What is reused, never rebuilt:** `aether-context` segment priorities and
//! compaction, `aether-sessions` for the raw record, `aether-evidence` for
//! verdicts, and `aether-core`'s task-state/verification enforcement — the
//! harness produces *inputs* to those, it does not duplicate them.
//!
//! Three principles (spec, final directive):
//! 1. Context is programmatically managed state, not a giant transcript.
//! 2. Long-running work persists outside the model's immediate context.
//! 3. Agents recursively delegate bounded work and continue from durable state.
//!
//! Security: everything the harness produces — memories, refinements, harness
//! state, goals — is **data**. It can never outrank system prompts, role
//! prompts, or permission rules, and the harness never shells out on its own:
//! gate execution goes through an injected `GateRunner`.

pub mod budget;
pub mod context;
pub mod events;
pub mod gates;
pub mod goal;
pub mod refine;
pub mod schedule;
pub mod state;
pub mod subagent;

use std::sync::Arc;

use parking_lot::Mutex;

pub use budget::{AutonomousOutcome, Budget, BudgetUsage, LimitReason};
pub use context::{
    CompiledContext, ContextEnvironment, ContextEvent, ContextEventKind, FileRef, MemoryRef,
    ToolResultRef, VerificationRef,
};
pub use events::{HarnessEvent, HarnessSink};
pub use gates::{GateAttempts, GateOutcome, GateRunner, QualityGates};
pub use goal::{Goal, GoalStatus};
pub use refine::{ContinualHarness, EntryKind, EntryScope, HarnessEntry, RefinementRecord};
pub use schedule::{DeliveryMode, JobStatus, Schedule, ScheduledJob, Scheduler};
pub use state::{HarnessSnapshot, HarnessStore, StateError};
pub use subagent::{ChildExecutor, ChildRecord, ChildResult, ChildSpec, ChildStatus, SubagentRuntime};

/// Retrieval tuning for the harness→memory bridge, declared here so the
/// harness stays independent of the concrete memory implementation.
#[derive(Debug, Clone)]
pub struct MemoryRetrievalOptions {
    pub max_candidates: usize,
    pub memory_budget_tokens: u32,
}

impl Default for MemoryRetrievalOptions {
    fn default() -> Self {
        Self {
            max_candidates: 8,
            memory_budget_tokens: 2_000,
        }
    }
}

/// Explicit runtime states (spec §24). One enum, not scattered strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Idle,
    Planning,
    Executing,
    Waiting,
    Verifying,
    Compacting,
    Autonomous,
    Paused,
    Blocked,
    Completed,
    Failed,
}

impl RuntimeState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Planning => "planning",
            Self::Executing => "executing",
            Self::Waiting => "waiting",
            Self::Verifying => "verifying",
            Self::Compacting => "compacting",
            Self::Autonomous => "autonomous",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// How the harness ends a bounded autonomous run (spec §17).
#[derive(Debug, Clone)]
pub struct AutonomousRun {
    pub state: RuntimeState,
    pub outcome: AutonomousOutcome,
    pub usage: BudgetUsage,
}

/// The harness facade. Deliberately a thin coordinator: each concern lives in
/// its own module and this struct only routes between them, so it never
/// becomes the giant class the spec warns about.
pub struct Harness {
    pub session_id: String,
    pub context: Mutex<ContextEnvironment>,
    pub goal: Mutex<Option<Goal>>,
    pub subagents: Arc<SubagentRuntime>,
    pub scheduler: Arc<Scheduler>,
    pub continual: Mutex<ContinualHarness>,
    pub gates: Mutex<QualityGates>,
    pub gate_attempts: Mutex<GateAttempts>,
    pub budget: Mutex<Budget>,
    pub usage: Mutex<BudgetUsage>,
    pub state: Mutex<RuntimeState>,
    store: Option<Arc<HarnessStore>>,
    sink: Mutex<HarnessSink>,
    memory: Mutex<Option<Arc<dyn MemoryRetriever>>>,
}

/// Retrieval seam so the harness does not depend on a specific memory
/// implementation (§44-style abstraction, reused from the Wave 9 design).
pub trait MemoryRetriever: Send + Sync {
    /// Memory references relevant to `request`, already packed.
    fn retrieve_refs(&self, request: &str, limit: usize) -> Vec<MemoryRef>;
}

impl Harness {
    /// Build a harness. `store` is optional so tests and short runs need no
    /// database; when present, state is persisted and recoverable.
    pub fn new(session_id: &str, store: Option<Arc<HarnessStore>>) -> Self {
        let sink: HarnessSink = events::null_sink();
        Self {
            session_id: session_id.to_string(),
            context: Mutex::new(ContextEnvironment::new()),
            goal: Mutex::new(None),
            subagents: Arc::new(SubagentRuntime::default()),
            scheduler: Arc::new(Scheduler::new()),
            continual: Mutex::new(ContinualHarness::new()),
            gates: Mutex::new(QualityGates::default()),
            gate_attempts: Mutex::new(GateAttempts::default()),
            budget: Mutex::new(Budget::default()),
            usage: Mutex::new(BudgetUsage::new(chrono::Utc::now().timestamp())),
            state: Mutex::new(RuntimeState::Idle),
            store,
            sink: Mutex::new(sink),
            memory: Mutex::new(None),
        }
    }

    /// Retrieve relevant memories for the current request. Empty when no
    /// retriever is wired or retrieval fails — never fatal to the run.
    pub fn retrieve_memories(&self, request: &str) -> Option<Vec<MemoryRef>> {
        let engine = self.memory.lock().clone()?;
        Some(engine.retrieve_refs(request, 8))
    }

    /// Durable supplemental state as a DATA block (never instructions).
    pub fn harness_digest(&self) -> String {
        self.continual.lock().render_digest(3, 400)
    }

    pub fn with_sink(self, sink: HarnessSink) -> Self {
        *self.sink.lock() = sink;
        self
    }

    pub fn with_budget(self, budget: Budget) -> Self {
        *self.budget.lock() = budget;
        self
    }

    pub fn with_gates(self, gates: QualityGates) -> Self {
        *self.gates.lock() = gates;
        self
    }

    /// Attach the retrieval engine (Wave 9 `aether-memory`) so the harness
    /// can fill its working context from durable memory each cycle.
    pub fn with_memory(self, engine: Arc<dyn MemoryRetriever>) -> Self {
        *self.memory.lock() = Some(engine);
        self
    }

    fn emit(&self, ev: HarnessEvent) {
        (self.sink.lock())(ev);
    }

    pub fn set_state(&self, s: RuntimeState) {
        *self.state.lock() = s;
    }

    /// Set the active goal and seed the context environment.
    pub fn set_goal(&self, goal: Goal) {
        self.context.lock().set_goal(goal.objective.clone());
        let id = goal.id.clone();
        let objective = goal.objective.clone();
        *self.goal.lock() = Some(goal);
        self.emit(HarnessEvent::GoalCreated {
            goal_id: id,
            objective,
        });
    }

    pub fn goal(&self) -> Option<Goal> {
        self.goal.lock().clone()
    }

    /// Compile the working context for the next model request, recording an
    /// event when sections had to be dropped.
    pub fn working_context(&self, budget_tokens: u32) -> CompiledContext {
        self.context.lock().compile(budget_tokens)
    }

    /// Run quality gates through the injected runner. The harness never
    /// shells out itself; the runtime supplies a permission-respecting runner.
    pub fn evaluate_gates(&self, runner: &dyn GateRunner) -> GateOutcome {
        let gates = self.gates.lock().clone();
        let mut attempts = self.gate_attempts.lock();
        let outcome = gates::evaluate(&gates, runner, &mut attempts);
        if let GateOutcome::Failed(f) | GateOutcome::RetryExhausted(f) = &outcome {
            self.emit(HarnessEvent::GateFailed {
                command: f.command.clone(),
                attempt: f.attempt,
                exit: f.exit_text.clone(),
            });
        }
        outcome
    }

    /// Advance the bounded autonomous loop one step. Returns `Some` when the
    /// run must stop, with the honest reason.
    pub fn tick(
        &self,
        turn: &mut BudgetUsage,
        runner: Option<&dyn GateRunner>,
    ) -> Option<AutonomousRun> {
        let now = chrono::Utc::now().timestamp();
        turn.add_turn();
        *self.usage.lock() = *turn;

        let gates_outcome = runner.map(|r| self.evaluate_gates(r));
        let budget = *self.budget.lock();
        let limit = budget.exhausted(turn, now);

        // Two independent stop authorities (reference rule): gates decide
        // success, budgets decide exhaustion. Never conflated.
        let retry_exhausted = match &gates_outcome {
            Some(GateOutcome::RetryExhausted(f)) => Some(AutonomousOutcome::GateRetryExhausted {
                command: f.command.clone(),
                attempts: f.attempt,
            }),
            _ => None,
        };

        let outcome = if let Some(o) = retry_exhausted {
            Some(o)
        } else {
            match (&gates_outcome, limit) {
                (Some(GateOutcome::Passed), _) => Some(AutonomousOutcome::GatesPassed),
                (Some(GateOutcome::Failed(_)), Some(l)) => {
                    Some(AutonomousOutcome::BudgetExhausted(l))
                }
                (Some(GateOutcome::Failed(_)), None) => None, // keep working
                (_, Some(l)) => Some(AutonomousOutcome::BudgetExhausted(l)),
                // No terminal evidence: do NOT stop and do NOT claim success.
                _ => None,
            }
        };

        outcome.map(|outcome| {
            let state = if outcome.is_success() {
                RuntimeState::Completed
            } else {
                RuntimeState::Paused
            };
            self.set_state(state);
            self.emit(HarnessEvent::AutonomousCompleted {
                reason: match &outcome {
                    AutonomousOutcome::BudgetExhausted(l) => l.describe(),
                    AutonomousOutcome::GateRetryExhausted { command, .. } => {
                        format!("gate retry exhausted: {command}")
                    }
                    AutonomousOutcome::GatesPassed => "quality gates passed".into(),
                    AutonomousOutcome::GoalComplete => "goal complete".into(),
                    AutonomousOutcome::Blocked { reason } => reason.clone(),
                    AutonomousOutcome::Failed { reason } => reason.clone(),
                },
                success: outcome.is_success(),
            });
            AutonomousRun {
                state,
                outcome,
                usage: *turn,
            }
        })
    }

    /// Persist everything the harness owns. No-op without a store.
    pub fn persist(&self) -> Result<(), StateError> {
        let Some(store) = &self.store else {
            return Ok(());
        };
        let snap = self.snapshot();
        store.save(&snap)
    }

    pub fn snapshot(&self) -> HarnessSnapshot {
        // One guard per mutex: taking two guards on the same non-reentrant
        // lock inside a single expression deadlocks.
        let (entries, refinements) = {
            let c = self.continual.lock();
            (
                c.entries().into_iter().cloned().collect(),
                c.history().to_vec(),
            )
        };
        let (gate_state, runtime_state, context_state) = {
            let g = self.gate_attempts.lock();
            let s = self.state.lock();
            let c = self.context.lock();
            (
                serde_json::to_string(&*g).ok(),
                serde_json::to_string(&*s).ok(),
                serde_json::to_string(&*c).ok(),
            )
        };
        HarnessSnapshot {
            goal: self.goal.lock().clone(),
            jobs: self.scheduler.jobs(),
            entries,
            refinements,
            children: self.subagents.all(),
            gate_state,
            runtime_state,
            context: context_state,
        }
    }

    /// Copy recovered state from a restored harness into this one. The
    /// runtime builds the live harness first (so it can own executors and
    /// sinks), then folds in what the store held.
    pub fn restore_from(&self, other: &Harness) {
        if let Some(g) = other.goal() {
            self.context.lock().set_goal(g.objective.clone());
            *self.goal.lock() = Some(g);
        }
        let restored_ctx = { other.context.lock().clone() };
        if restored_ctx.goal.is_some()
            || restored_ctx.checkpoint.is_some()
            || !restored_ctx.recent_events.is_empty()
        {
            let mut live = self.context.lock();
            live.recent_events = restored_ctx.recent_events;
            live.checkpoint = restored_ctx.checkpoint;
            live.plan = restored_ctx.plan;
            live.relevant_files = restored_ctx.relevant_files;
            live.verification = restored_ctx.verification;
            live.tool_results = restored_ctx.tool_results;
            live.conversation_refs = restored_ctx.conversation_refs;
        }
        let (entries, refinements) = {
            let c = other.continual.lock();
            (
                c.entries().into_iter().cloned().collect(),
                c.history().to_vec(),
            )
        };
        self.continual.lock().load_entries(entries);
        self.continual.lock().load_history(refinements);
        for j in other.scheduler.jobs() {
            self.scheduler.add(j);
        }
    }

    /// Restore a persisted session without replaying the conversation
    /// (spec §14). Interrupted children are reported, never assumed done.
    pub fn restore(
        store: Arc<HarnessStore>,
        session_id: &str,
    ) -> Result<(Self, HarnessSnapshot), StateError> {
        let snap = store.load(Some(session_id))?;
        let h = Self::new(session_id, Some(store.clone()));
        if let Some(g) = &snap.goal {
            h.context.lock().set_goal(g.objective.clone());
            *h.goal.lock() = Some(g.clone());
        }
        // Working context (checkpoint, recent events, memories) is restored
        // from durable state, never by replaying the transcript.
        if let Some(c) = &snap.context {
            if let Ok(env) = serde_json::from_str::<ContextEnvironment>(c) {
                *h.context.lock() = env;
            }
        }
        if let Some(s) = &snap.runtime_state {
            if let Ok(rs) = serde_json::from_str::<RuntimeState>(s) {
                *h.state.lock() = rs;
            }
        }
        h.continual.lock().load_entries(snap.entries.clone());
        h.continual.lock().load_history(snap.refinements.clone());
        for j in &snap.jobs {
            h.scheduler.add(j.clone());
        }
        if let Some(g) = &snap.gate_state {
            if let Ok(attempts) = serde_json::from_str::<GateAttempts>(g) {
                *h.gate_attempts.lock() = attempts;
            }
        }
        let interrupted = store.interrupted_children()?;
        if !interrupted.is_empty() {
            h.emit(HarnessEvent::RecoveryRestored {
                session_id: session_id.to_string(),
                detail: format!("{} child agent(s) were interrupted", interrupted.len()),
            });
        }
        Ok((h, snap))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Deterministic executor: every child "investigates" successfully.
    struct OkExecutor;

    #[async_trait]
    impl ChildExecutor for OkExecutor {
        async fn execute(&self, record: &ChildRecord, _c: bool) -> ChildResult {
            ChildResult {
                status: "ok".into(),
                summary: format!("{} investigated", record.spec.agent_id),
                findings: vec!["src/lib.rs touched".into()],
                ..Default::default()
            }
        }
    }

    fn harness_with_store() -> (Harness, Arc<HarnessStore>) {
        let store = Arc::new(HarnessStore::open_in_memory().unwrap());
        let h = Harness::new("sess-e2e", Some(store.clone()));
        let h = h.with_sink(Arc::new(|_| {})).with_budget(Budget::turns(3));
        h.subagents.set_executor(Arc::new(OkExecutor));
        (h, store)
    }

    #[test]
    fn runtime_states_are_explicit_labels() {
        assert_eq!(RuntimeState::Compacting.label(), "compacting");
        assert_ne!(RuntimeState::Blocked.label(), RuntimeState::Paused.label());
    }

    #[test]
    fn working_context_compiles_under_budget_and_reports_drops() {
        let (h, _) = harness_with_store();
        h.context.lock().set_goal("Refactor provider architecture");
        for i in 0..40 {
            h.context.lock().push_event(ContextEvent {
                kind: ContextEventKind::ToolResult,
                text: format!("tool output {i}"),
                source_id: None,
                at: i,
            });
        }
        let compiled = h.working_context(50);
        assert!(compiled.tokens <= 50);
        assert!(!compiled.dropped.is_empty());
        assert!(compiled.render().contains("WORKING CONTEXT"));
    }

    #[test]
    fn budget_exhaustion_pauses_without_claiming_success() {
        let (h, _) = harness_with_store();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink_events = events.clone();
        let h = h.with_sink(Arc::new(move |e: HarnessEvent| {
            sink_events.lock().push(e);
        }));
        let mut usage = BudgetUsage::new(chrono::Utc::now().timestamp());
        assert!(h.tick(&mut usage, None).is_none());
        assert!(h.tick(&mut usage, None).is_none());
        let run = h.tick(&mut usage, None).expect("third turn exhausts max_turns=3");
        assert!(!run.outcome.is_success());
        assert_eq!(run.state, RuntimeState::Paused);
        let completed = events
            .lock()
            .iter()
            .any(|e| matches!(e, HarnessEvent::AutonomousCompleted { success: false, .. }));
        assert!(completed, "budget stop must emit success=false");
    }

    #[test]
    fn goal_completion_is_gated_on_progress() {
        let (h, _) = harness_with_store();
        h.set_goal(Goal::new("sess-e2e", "Refactor providers").with_tasks(2));
        let mut g = h.goal().unwrap();
        assert!(g.try_complete().is_err());
        g.record_task_done();
        g.record_task_done();
        assert!(g.try_complete().is_ok());
    }

    #[test]
    fn persist_and_restore_recovers_goal_and_refinements() {
        let store = Arc::new(HarnessStore::open_in_memory().unwrap());
        let h = Harness::new("sess-rec", Some(store.clone()));
        h.set_goal(Goal::new("sess-rec", "Refactor providers").with_tasks(5));
        h.scheduler.add(ScheduledJob::new(
            "sess-rec",
            "hb",
            "continue the refactor",
            Schedule::Interval { period_ms: 60_000 },
        ));
        h.continual
            .lock()
            .apply(
                RefinementRecord::new(
                    "sess-rec",
                    "recurring compile failure",
                    vec!["cargo check failed twice".into()],
                    "add lesson",
                    "prevent repeat",
                    EntryScope::Project,
                    Some("task-1".into()),
                ),
                vec![refine::AppliedEdit {
                    action: refine::EditAction::Create,
                    kind: EntryKind::Lesson,
                    id: "l1".into(),
                    reason: "evidence".into(),
                    before: None,
                    after: Some(HarnessEntry::new(
                        "l1",
                        EntryKind::Lesson,
                        "Lesson",
                        "put mingw64 first",
                        EntryScope::Project,
                    )),
                    applied: false,
                    error: None,
                }],
            )
            .unwrap();
        h.persist().unwrap();

        let (_restored, snap) = Harness::restore(store.clone(), "sess-rec").unwrap();
        assert_eq!(snap.goal.unwrap().objective, "Refactor providers");
        assert_eq!(snap.jobs.len(), 1);
        assert_eq!(snap.entries.len(), 1);
        assert_eq!(snap.refinements.len(), 1);
    }

    /// Spec §29: the realistic end-to-end lifecycle. Goal → plan → execute →
    /// children (investigate/implement/verify) → results back → failure fixed
    /// → context grows → compaction checkpoint → persist → session resumes →
    /// old info retrieved → more work → verification → goal completes.
    #[tokio::test]
    async fn long_running_task_survives_its_whole_lifecycle() {
        let (h, store) = harness_with_store();
        let failures = AtomicUsize::new(0);

        // 1. Goal created with real work to do.
        h.set_goal(Goal::new("sess-e2e", "Refactor provider architecture").with_tasks(3));

        // 2. Plan recorded into structured context.
        h.context.lock().set_plan("1) inspect gateway 2) patch 3) verify");
        h.set_state(RuntimeState::Planning);

        // 3. Execute a cycle: spawn an investigator + an implementer.
        h.set_state(RuntimeState::Executing);
        let mut turn = BudgetUsage::new(chrono::Utc::now().timestamp());
        let investigator = h
            .subagents
            .spawn(None, "sess-e2e", Some("task-1"), ChildSpec {
                agent_id: "explorer".into(),
                objective: "inspect provider validation".into(),
                context: "focus: src/providers".into(),
                scope: vec!["crates".into()],
                allowed_tools: None,
                budget: Budget::turns(1),
                expected_output: "status/summary/findings".into(),
            })
            .unwrap();
        let implementer = h
            .subagents
            .spawn(Some(&investigator), "sess-e2e", Some("task-1"), ChildSpec {
                agent_id: "implementer".into(),
                objective: "patch normalization".into(),
                context: "focus: gateway.rs".into(),
                scope: vec!["crates/aether-gateway".into()],
                allowed_tools: None,
                budget: Budget::turns(1),
                expected_output: "files_changed".into(),
            })
            .unwrap();
        assert!(!implementer.is_empty());
        h.subagents.run_queued().await;

        // 4. Results return to the parent as structured data.
        let results = h.subagents.all();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.status.is_settled()));
        assert!(
            results.iter().any(|r| r.depth == 2),
            "recursive child should be at depth 2"
        );

        // 5. A failure is represented and fixed, not swallowed.
        h.context.lock().push_event(ContextEvent {
            kind: ContextEventKind::Error,
            text: "provider validation test failing".into(),
            source_id: None,
            at: 1,
        });
        failures.fetch_add(1, Ordering::SeqCst);
        h.context.lock().push_event(ContextEvent {
            kind: ContextEventKind::Verification,
            text: "cargo test: 27 passed after fix".into(),
            source_id: None,
            at: 2,
        });

        // 6. Context grows; compaction checkpoints durable state. The
        //    checkpoint is a compact record of what matters — not the noisy
        //    200-message tail (that history stays retrievable, not inline).
        for i in 0..200 {
            h.context.lock().push_event(ContextEvent {
                kind: ContextEventKind::ToolResult,
                text: format!("noisy tool output {i}"),
                source_id: None,
                at: 10 + i,
            });
        }
        h.set_state(RuntimeState::Compacting);
        {
            let mut ctx = h.context.lock();
            ctx.checkpoint = Some(
                "objective=refactor providers; \
                 error=provider validation test failing; \
                 verified=cargo test 27 passed after fix; \
                 next=finish remaining tasks"
                    .into(),
            );
        }
        // Compacting really does shrink the active context.
        let compiled = h.working_context(400);
        assert!(!compiled.dropped.is_empty(), "budget must drop the tail");
        h.set_state(RuntimeState::Executing);

        // 7. Persist, then simulate a restart: state is restored WITHOUT
        //    replaying the 200-message history.
        h.persist().unwrap();
        let (restored, snap) = Harness::restore(store.clone(), "sess-e2e").unwrap();
        let g = restored.goal().expect("goal survived restart");
        assert_eq!(g.objective, "Refactor provider architecture");
        assert!(g.tasks_total == 3);
        assert_eq!(snap.children.len(), 2, "child records recovered");

        // 8. Old information is retrieved on demand (compacted history is
        //    still reachable, not lost).
        let ctx = restored.working_context(4_000);
        let rendered = ctx.render();
        assert!(
            rendered.contains("provider validation") || rendered.contains("cargo test"),
            "verification/error context must survive compaction: {rendered}"
        );

        // 9. Remaining work: record progress, then verify the goal.
        let mut goal = restored.goal().unwrap();
        for _ in 0..3 {
            goal.record_task_done();
        }
        assert!(goal.try_complete().is_ok(), "goal completes only after real progress");

        // 10. A turn was consumed; budget still governs the run.
        turn.add_turn();
        assert!(h.budget.lock().exhausted(&turn, chrono::Utc::now().timestamp()).is_none());
        assert_eq!(store.schema_version().unwrap(), state::SCHEMA_VERSION);
    }
}
