//! Typed hook inputs and outputs.
//!
//! Every extension point has a fixed `Input` and `Output` type. Plugins
//! receive mutable references to both; the chain threads `Output` through
//! every plugin in priority order, then the runtime commits the final
//! `Output` as the resolved state.
//!
//! `Before*` hooks may short-circuit by returning [`HookOutcome::Halt`].
//! `After*` hooks are observability only — they cannot abort.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The decision a single plugin handler returns. The registry composes these
/// into a final chain decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HookOutcome {
    /// Continue down the chain with the current `Output`.
    Continue,
    /// Stop the chain. Subsequent plugins are not called. The runtime sees
    /// this as a hard veto and refuses to perform the underlying action.
    Halt { reason: String },
}

// ---- BeforeModelRequest -----------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRequestInput {
    pub model: String,
    pub system: String,
    pub messages: Vec<ModelMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMessage {
    pub role: String,
    pub content: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelRequestOutput {
    /// If `Some(true)`, the request is aborted (e.g. policy plugin rejected).
    pub abort: Option<bool>,
    /// If `Some(reason)`, the request is aborted with this reason.
    pub abort_reason: Option<String>,
    /// Free-form metadata plugins can attach for audit (e.g. redacted tokens).
    pub metadata: std::collections::HashMap<String, String>,
}

pub type BeforeModelRequest = (ModelRequestInput, ModelRequestOutput);

// ---- AfterModelResponse -----------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelResponseInput {
    pub model: String,
    pub agent_id: Option<String>,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelResponseOutput {
    /// Free-form metadata (e.g. cost calculation, plugin annotations).
    pub metadata: std::collections::HashMap<String, String>,
}

pub type AfterModelResponse = (ModelResponseInput, ModelResponseOutput);

// ---- BeforeToolExecute / AfterToolExecute -----------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecuteInput {
    pub tool: String,
    pub args: serde_json::Value,
    pub agent_id: Option<String>,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolExecuteOutput {
    /// Set to `Some(true)` to abort the tool call (e.g. plugin denied it).
    pub abort: Option<bool>,
    pub abort_reason: Option<String>,
    /// If set, the tool's args are replaced with this value (sanitisation).
    pub rewritten_args: Option<serde_json::Value>,
    /// Free-form metadata (audit trail entry, redacted paths, etc.).
    pub metadata: std::collections::HashMap<String, String>,
}

pub type BeforeToolExecute = (ToolExecuteInput, ToolExecuteOutput);
pub type AfterToolExecute = (ToolExecuteInput, ToolExecuteOutput);

// ---- BeforeContextCompact / AfterContextCompact -----------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextCompactInput {
    pub agent_id: String,
    pub kind: String,
    pub threshold_pct: f32,
    pub current_tokens: u32,
    pub context_window: u32,
    pub pinned_segment_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextCompactOutput {
    /// If `Some(true)`, compaction is cancelled.
    pub abort: Option<bool>,
    pub abort_reason: Option<String>,
    /// Additional segments to pin (will not be compacted).
    pub extra_pins: Vec<String>,
    pub metadata: std::collections::HashMap<String, String>,
}

pub type BeforeContextCompact = (ContextCompactInput, ContextCompactOutput);
pub type AfterContextCompact = (ContextCompactInput, ContextCompactOutput);

// ---- OnAgentSpawn / OnAgentComplete -----------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpawnHookInput {
    pub agent_id: String,
    pub role: String,
    pub parent: Option<String>,
    pub depth: u32,
    pub task: String,
    pub model: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentSpawnHookOutput {
    /// Mutated task description; plugins may rewrite / annotate the task.
    pub task: String,
    /// Mutated model key; plugins may override (e.g. for routing).
    pub model: String,
    /// Extra capability tags attached to this agent.
    pub capabilities: Vec<String>,
    pub metadata: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCompleteInput {
    pub agent_id: String,
    pub role: String,
    pub status: String,
    pub summary: String,
    pub findings: Vec<String>,
    pub files: Vec<String>,
    pub latency_ms: u64,
    pub tokens_in: u32,
    pub tokens_out: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentCompleteOutput {
    pub verified: bool,
    pub confidence: f32,
    pub metadata: std::collections::HashMap<String, String>,
}

// ---- OnSnapshotCreate / OnSnapshotRestore -----------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotCreateInput {
    pub snapshot_id: String,
    pub session_id: String,
    pub agent_id: Option<String>,
    pub trigger: String,
    pub label: Option<String>,
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotCreateOutput {
    /// Mutated label (e.g. plugin enriches with reason).
    pub label: Option<String>,
    pub tags: Vec<String>,
    pub metadata: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRestoreInput {
    pub snapshot_id: String,
    pub session_id: String,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotRestoreOutput {
    /// If `Some(true)`, restore is aborted.
    pub abort: Option<bool>,
    pub abort_reason: Option<String>,
    pub metadata: std::collections::HashMap<String, String>,
}

// ---- OnSessionStart / OnSessionEnd -----------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartInput {
    pub session_id: String,
    pub cwd: PathBuf,
    pub resumed: bool,
    pub model: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionStartOutput {
    pub metadata: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEndInput {
    pub session_id: String,
    pub exit_reason: String,
    pub duration_secs: u64,
    pub success: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionEndOutput {
    pub metadata: std::collections::HashMap<String, String>,
}

// ---- Hook trait -------------------------------------------------------------

use async_trait::async_trait;

/// A single hook handler — a `Plugin`'s typed callback for one extension point.
#[async_trait]
pub trait Hook<Input, Output>: Send + Sync + 'static {
    async fn run(&self, input: &mut Input, output: &mut Output) -> HookOutcome;
}
