//! Typed events for the plugin event bus.
//!
//! Events are emitted by the runtime at extension points (snapshot created,
//! agent spawned, tool executed, …) and fanned out to all registered
//! subscribers. Subscribers cannot mutate the event payload; use a hook
//! for that.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Strongly-typed event kinds. Add new variants as new extension points are
/// introduced; downstream subscribers can pattern-match exhaustively.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    /// A snapshot was created.
    SnapshotCreated,
    /// A snapshot was restored.
    SnapshotRestored,
    /// A new agent was spawned.
    AgentSpawned,
    /// An agent completed (success or failure).
    AgentCompleted,
    /// A tool was executed.
    ToolExecuted,
    /// A model request was sent.
    ModelRequested,
    /// A model response was received.
    ModelResponded,
    /// Context was compacted for an agent.
    ContextCompacted,
    /// A provider/model was validated.
    ProviderValidated,
    /// A permission decision was made.
    PermissionDecided,
    /// Free-form plugin-defined kind (escape hatch).
    Custom(String),
}

impl EventKind {
    pub fn name(&self) -> &str {
        match self {
            EventKind::SnapshotCreated => "snapshot_created",
            EventKind::SnapshotRestored => "snapshot_restored",
            EventKind::AgentSpawned => "agent_spawned",
            EventKind::AgentCompleted => "agent_completed",
            EventKind::ToolExecuted => "tool_executed",
            EventKind::ModelRequested => "model_requested",
            EventKind::ModelResponded => "model_responded",
            EventKind::ContextCompacted => "context_compacted",
            EventKind::ProviderValidated => "provider_validated",
            EventKind::PermissionDecided => "permission_decided",
            EventKind::Custom(s) => s,
        }
    }
}

/// A typed event payload. `kind` is the discriminator; `payload` is a JSON
/// blob whose shape depends on `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub kind: EventKind,
    pub payload: Value,
}

/// Subscribers receive every event published to the registry.
#[async_trait]
pub trait EventSubscriber: Send + Sync + 'static {
    async fn on_event(&self, event: &Event) -> anyhow::Result<()>;
}
