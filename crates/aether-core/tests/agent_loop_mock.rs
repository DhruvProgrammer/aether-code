//! Integration test scaffolding for the agent loop with `MockProvider` and
//! `MockWorkspace`. The full agent-loop wire-in via the executor is
//! a later step; this test verifies the mock infrastructure contracts.

use aether_core::task_state::TaskState;
use aether_core::testing::{MockProvider, MockResponse, MockWorkspace};
use aether_models::Message;
use aether_tools::workspace::Workspace;
use std::path::PathBuf;

#[tokio::test]
async fn mock_provider_serves_scripted_responses() {
    let provider = MockProvider::new(vec![
        MockResponse::text("turn 1"),
        MockResponse::tool_call("read_file", serde_json::json!({"path": "a.rs"})),
        MockResponse::text("turn 3"),
    ]);
    let mut ws = MockWorkspace::new(PathBuf::from("/tmp/ws"));
    ws.seed("a.rs", "fn main() {}");
    // Sanity-check the mock workspace surface
    let read = ws.read(&PathBuf::from("a.rs"), 0, 0, 0).await.unwrap();
    assert!(read.text.contains("main"));
    let found = ws.find_files("a.rs", 4).await.unwrap();
    assert_eq!(found.len(), 1);
    // The mock provider starts with no recorded requests.
    assert!(provider.recorded().is_empty());
    let _ = (Message::default(),);
}

#[tokio::test]
async fn tool_output_truncates_oversize() {
    use aether_context::budget::{truncate_tool_output, TOOL_OUTPUT_DEFAULT_MAX_BYTES};
    let big = "x".repeat(TOOL_OUTPUT_DEFAULT_MAX_BYTES + 100);
    let (out, cut) = truncate_tool_output(&big, TOOL_OUTPUT_DEFAULT_MAX_BYTES);
    assert!(cut);
    assert!(out.starts_with(&"x".repeat(TOOL_OUTPUT_DEFAULT_MAX_BYTES)));
    assert!(out.contains("truncated"));
}

#[tokio::test]
async fn task_state_machine_full_lifecycle() {
    use aether_core::task_state::LlmRole;
    use aether_core::task_state::TaskStateMachine;
    let mut tsm = TaskStateMachine::new("task-1", "session-1");
    assert_eq!(tsm.state(), TaskState::Created);
    tsm.transition(TaskState::Understanding, LlmRole::Reviewer, "go").unwrap();
    tsm.transition(TaskState::Planning, LlmRole::Planner, "go").unwrap();
    tsm.transition(TaskState::PlanReady, LlmRole::Planner, "ok").unwrap();
    tsm.transition(TaskState::Executing, LlmRole::Executor, "build").unwrap();
    tsm.transition(TaskState::Reviewing, LlmRole::Reviewer, "check").unwrap();
    tsm.transition(TaskState::Verifying, LlmRole::Reviewer, "verify").unwrap();
    tsm.add_verification_evidence("tests", "pass", "all tests pass", None);
    tsm.add_verification_evidence("build", "pass", "cargo build ok", None);
    tsm.conclude_verification(true);
    tsm.transition(TaskState::Completed, LlmRole::Reviewer, "done").unwrap();
    assert_eq!(tsm.state(), TaskState::Completed);
    assert!(tsm.record.transitions.len() >= 6);
    let _ = (Message::default(),);
}
