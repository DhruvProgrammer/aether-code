//! Quality gates (spec §18, §22).
//!
//! Reuse ledger: AETHER's `TaskStateMachine` already *refuses* completion
//! without verification evidence, and `aether-evidence` aggregates verdicts.
//! A harness gate therefore does not duplicate that — it runs real commands
//! and produces evidence, which the state machine then enforces.
//!
//! Two rules borrowed from the reference harness because they prevent the
//! most common way quality gates lie:
//!
//! * **First failure wins** — gates run in order and stop at the first
//!   failure, so the reported failure is the real one.
//! * **Never re-run over an unchanged workspace** — a failing gate is not
//!   retried until the workspace fingerprint changes, which forces real work
//!   instead of idle retry loops.
//!
//! A gate may only claim what it actually ran: timeouts and spawn errors are
//! failures, never silent passes.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Cap on surfaced gate output so a 20k-line log cannot flood the context.
pub const MAX_GATE_OUTPUT_CHARS: usize = 6_000;

/// Configured gates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QualityGates {
    /// Commands that must all succeed, run in order.
    pub commands: Vec<String>,
    pub max_retries: u32,
    pub timeout_secs: u64,
}

impl QualityGates {
    pub fn is_configured(&self) -> bool {
        !self.commands.is_empty()
    }
}

/// What one command produced.
#[derive(Debug, Clone)]
pub struct GateRun {
    pub exit_code: Option<i32>,
    pub output: String,
    pub timed_out: bool,
    pub error: Option<String>,
}

impl GateRun {
    pub fn ok(output: &str) -> Self {
        Self {
            exit_code: Some(0),
            output: output.to_string(),
            timed_out: false,
            error: None,
        }
    }

    pub fn failed(exit_code: Option<i32>, output: &str, error: Option<String>) -> Self {
        Self {
            exit_code,
            output: output.to_string(),
            timed_out: false,
            error,
        }
    }

    /// Exit-code-style text, matching how the failure is reported to the model.
    pub fn exit_text(&self) -> String {
        if self.timed_out {
            return "timed out".into();
        }
        if let Some(e) = &self.error {
            return format!("failed to run: {e}");
        }
        match self.exit_code {
            Some(0) => "passed".into(),
            Some(c) => format!("exited with code {c}"),
            None => "no exit status".into(),
        }
    }

    pub fn is_success(&self) -> bool {
        self.exit_code == Some(0) && !self.timed_out && self.error.is_none()
    }
}

/// Execution seam. The runtime supplies an implementation that respects the
/// permission engine — the harness never shells out on its own (spec §27).
pub trait GateRunner: Send + Sync {
    fn run(&self, command: &str, timeout_secs: u64) -> GateRun;
    /// Fingerprint of the workspace state a gate ran against.
    fn workspace_fingerprint(&self) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateFailure {
    pub command: String,
    pub attempt: u32,
    pub exit_text: String,
    pub output: String,
}

/// Result of evaluating all gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    /// No gates configured — the caller must not claim verification.
    NotConfigured,
    Passed,
    /// Failed, retries remain. `attempt` is the new count.
    Failed(GateFailure),
    /// Failed and the retry window is spent.
    RetryExhausted(GateFailure),
}

/// Attempt bookkeeping, persisted with the harness state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GateAttempts {
    pub per_command: HashMap<String, u32>,
    pub last_failure: Option<GateFailure>,
    /// Workspace fingerprint at the time of the last failure.
    pub last_failure_fingerprint: Option<String>,
}

impl GateAttempts {
    pub fn attempt_of(&self, command: &str) -> u32 {
        self.per_command.get(command).copied().unwrap_or(0)
    }
}

fn truncate(s: &str) -> String {
    if s.chars().count() <= MAX_GATE_OUTPUT_CHARS {
        return s.to_string();
    }
    let head: String = s.chars().take(MAX_GATE_OUTPUT_CHARS).collect();
    format!("{head}\n…[truncated]")
}

/// Evaluate gates against the runner. First failure wins; unchanged
/// workspaces are not retried.
pub fn evaluate(
    gates: &QualityGates,
    runner: &dyn GateRunner,
    attempts: &mut GateAttempts,
) -> GateOutcome {
    if !gates.is_configured() {
        return GateOutcome::NotConfigured;
    }
    for command in &gates.commands {
        let fingerprint = runner.workspace_fingerprint();
        // Anti-idle: same command, same workspace, already failed → don't run.
        if let Some(prev) = &attempts.last_failure {
            if &prev.command == command
                && attempts.last_failure_fingerprint.as_deref() == Some(fingerprint.as_str())
            {
                let attempt = prev.attempt + 1;
                let failure = GateFailure {
                    command: command.clone(),
                    attempt,
                    exit_text: "not rerun: workspace unchanged since previous failed gate".into(),
                    output: prev.output.clone(),
                };
                return if attempt <= gates.max_retries {
                    attempts
                        .per_command
                        .insert(command.clone(), attempt);
                    attempts.last_failure = Some(failure.clone());
                    attempts.last_failure_fingerprint = Some(fingerprint);
                    GateOutcome::Failed(failure)
                } else {
                    attempts.last_failure = Some(failure.clone());
                    GateOutcome::RetryExhausted(failure)
                };
            }
        }

        let run = runner.run(command, gates.timeout_secs);
        if run.is_success() {
            attempts.per_command.insert(command.clone(), 0);
            attempts.last_failure = None;
            attempts.last_failure_fingerprint = None;
            continue;
        }
        let attempt = attempts.attempt_of(command) + 1;
        let failure = GateFailure {
            command: command.clone(),
            attempt,
            exit_text: run.exit_text(),
            output: truncate(&format!(
                "{}{}",
                run.output,
                run.error
                    .as_ref()
                    .map(|e| format!("\nerror: {e}"))
                    .unwrap_or_default()
            )),
        };
        attempts.per_command.insert(command.clone(), attempt);
        attempts.last_failure = Some(failure.clone());
        attempts.last_failure_fingerprint = Some(fingerprint);
        return if attempt <= gates.max_retries {
            GateOutcome::Failed(failure)
        } else {
            GateOutcome::RetryExhausted(failure)
        };
    }
    attempts.last_failure = None;
    attempts.last_failure_fingerprint = None;
    GateOutcome::Passed
}

/// Failure text injected as the next turn's input, so the model sees exactly
/// what failed and why.
pub fn render_failure_continuation(f: &GateFailure, max_retries: u32) -> String {
    format!(
        "[quality-gate-failed]\n\nQuality gate failed (attempt {}/{}): `{}` {}.\nOutput:\n{}\n\nContinue working. Fix the failure, then produce terminal evidence.",
        f.attempt,
        max_retries.max(f.attempt),
        f.command,
        f.exit_text,
        f.output
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ScriptedRunner {
        results: std::sync::Mutex<std::collections::VecDeque<GateRun>>,
        calls: std::sync::Mutex<Vec<String>>,
        fingerprint: std::sync::Mutex<String>,
    }

    impl ScriptedRunner {
        fn new(results: Vec<GateRun>) -> Self {
            Self {
                results: std::sync::Mutex::new(results.into_iter().collect()),
                calls: std::sync::Mutex::new(Vec::new()),
                fingerprint: std::sync::Mutex::new("fp-1".into()),
            }
        }
    }

    impl GateRunner for ScriptedRunner {
        fn run(&self, command: &str, _timeout: u64) -> GateRun {
            self.calls.lock().unwrap().push(command.to_string());
            self.results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| GateRun::ok(""))
        }
        fn workspace_fingerprint(&self) -> String {
            self.fingerprint.lock().unwrap().clone()
        }
    }

    fn gates(cmds: &[&str], max_retries: u32) -> QualityGates {
        QualityGates {
            commands: cmds.iter().map(|s| s.to_string()).collect(),
            max_retries,
            timeout_secs: 60,
        }
    }

    #[test]
    fn no_gates_is_not_a_pass() {
        let r = ScriptedRunner::new(vec![]);
        let mut a = GateAttempts::default();
        assert_eq!(
            evaluate(&QualityGates::default(), &r, &mut a),
            GateOutcome::NotConfigured
        );
    }

    #[test]
    fn all_passing_gates_pass_in_order() {
        let r = ScriptedRunner::new(vec![GateRun::ok("ok"), GateRun::ok("ok")]);
        let mut a = GateAttempts::default();
        assert_eq!(evaluate(&gates(&["cargo test", "cargo clippy"], 2), &r, &mut a), GateOutcome::Passed);
        assert_eq!(*r.calls.lock().unwrap(), vec!["cargo test", "cargo clippy"]);
    }

    #[test]
    fn first_failure_wins_and_stops() {
        let r = ScriptedRunner::new(vec![GateRun::ok("ok"), GateRun::failed(Some(1), "2 tests failed", None)]);
        let mut a = GateAttempts::default();
        match evaluate(&gates(&["cargo test", "cargo clippy"], 2), &r, &mut a) {
            GateOutcome::Failed(f) => {
                assert_eq!(f.command, "cargo clippy");
                assert_eq!(f.attempt, 1);
                assert!(f.output.contains("2 tests failed"));
            }
            other => panic!("expected failure, got {other:?}"),
        }
        // The gate after the failure never ran.
        assert_eq!(*r.calls.lock().unwrap(), vec!["cargo test", "cargo clippy"]);
    }

    #[test]
    fn unchanged_workspace_is_not_retried() {
        let r = ScriptedRunner::new(vec![GateRun::failed(Some(1), "boom", None)]);
        let mut a = GateAttempts::default();
        let g = gates(&["cargo test"], 3);
        assert!(matches!(evaluate(&g, &r, &mut a), GateOutcome::Failed(_)));
        // Second evaluation with the same fingerprint must not invoke the runner.
        match evaluate(&g, &r, &mut a) {
            GateOutcome::Failed(f) => assert!(f.exit_text.contains("not rerun")),
            other => panic!("expected failure, got {other:?}"),
        }
        assert_eq!(r.calls.lock().unwrap().len(), 1, "runner invoked despite no workspace change");
    }

    #[test]
    fn retries_exhaust_into_retry_exhausted() {
        let r = ScriptedRunner::new(vec![
            GateRun::failed(Some(1), "1", None),
            GateRun::failed(Some(1), "2", None),
        ]);
        let mut a = GateAttempts::default();
        let g = gates(&["cargo test"], 2);
        // attempt 1 and attempt 2 (the allowed retries) are both failures.
        assert!(matches!(evaluate(&g, &r, &mut a), GateOutcome::Failed(_)));
        *r.fingerprint.lock().unwrap() = "fp-2".into();
        match evaluate(&g, &r, &mut a) {
            GateOutcome::Failed(f) => assert_eq!(f.attempt, 2),
            other => panic!("expected second failure, got {other:?}"),
        }
        // Third failure exhausts the retry window (and is not re-run, because
        // the workspace did not change).
        match evaluate(&g, &r, &mut a) {
            GateOutcome::RetryExhausted(f) => {
                assert_eq!(f.attempt, 3);
                assert!(f.exit_text.contains("not rerun"));
            }
            other => panic!("expected exhaustion, got {other:?}"),
        }
    }

    #[test]
    fn timeouts_and_spawn_errors_are_failures() {
        let r = ScriptedRunner::new(vec![
            GateRun {
                exit_code: None,
                output: "partial".into(),
                timed_out: true,
                error: None,
            },
            GateRun::failed(None, "", Some("command not found".into())),
        ]);
        let mut a = GateAttempts::default();
        let g = gates(&["cargo test"], 1);
        match evaluate(&g, &r, &mut a) {
            GateOutcome::Failed(f) => assert_eq!(f.exit_text, "timed out"),
            other => panic!("expected failure, got {other:?}"),
        }
        *r.fingerprint.lock().unwrap() = "fp-2".into();
        match evaluate(&g, &r, &mut a) {
            GateOutcome::RetryExhausted(f) => {
                assert!(f.exit_text.contains("command not found"));
                assert!(f.output.contains("command not found"));
            }
            other => panic!("expected exhaustion, got {other:?}"),
        }
    }

    #[test]
    fn failure_continuation_names_the_command() {
        let f = GateFailure {
            command: "cargo test".into(),
            attempt: 1,
            exit_text: "exited with code 101".into(),
            output: "2 failed".into(),
        };
        let s = render_failure_continuation(&f, 3);
        assert!(s.contains("cargo test"));
        assert!(s.contains("1/3"));
        assert!(s.contains("2 failed"));
    }
}
