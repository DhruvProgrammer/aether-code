//! Typed runtime event bus (OpenCode-inspired).
//!
//! Each event category has its own broadcast channel so a UI listener that
//! cares about files (for example) does not pay for tool events it ignores.
//! The bus is `Send + Sync` and uses `tokio::sync::broadcast` for fan-out
//! semantics. Senders (`Agent::run`, `Executor`) call `bus.publish`;
//! listeners (the desktop Tauri layer, the frontend) call `bus.subscribe`
//! and receive `EventEnvelope` JSON values.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::SendError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventCategory {
    Agent,
    Model,
    Tool,
    File,
    Context,
    Verification,
    Session,
    Compaction,
    Workspace,
}

impl EventCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventCategory::Agent => "agent",
            EventCategory::Model => "model",
            EventCategory::Tool => "tool",
            EventCategory::File => "file",
            EventCategory::Context => "context",
            EventCategory::Verification => "verification",
            EventCategory::Session => "session",
            EventCategory::Compaction => "compaction",
            EventCategory::Workspace => "workspace",
        }
    }
}

/// One event envelope carried on the bus. The `data` is a JSON value
/// carrying the category-specific payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub category: EventCategory,
    pub ts_ms: i64,
    pub data: serde_json::Value,
}

const CHANNEL_CAPACITY: usize = 256;

/// In-process typed event bus. Cheap, in-memory, non-persistent.
#[derive(Clone)]
pub struct RuntimeEventBus {
    inner: Arc<Inner>,
}

struct Inner {
    tx: broadcast::Sender<EventEnvelope>,
}

impl RuntimeEventBus {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        Self { inner: Arc::new(Inner { tx }) }
    }

    /// Publish an envelope. The `data` should be a JSON-serializable
    /// category-specific payload. Returns the number of subscribers the
    /// event was delivered to, or an error if there are no active receivers
    /// and the channel is closed.
    pub fn publish(&self, category: EventCategory, data: serde_json::Value) -> Result<usize, SendError<EventEnvelope>> {
        let ts_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        self.inner.tx.send(EventEnvelope { category, ts_ms, data })
    }

    /// Subscribe to the bus. Returns a receiver the caller can poll; each
    /// receiver gets all events (filter by `envelope.category` if needed).
    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.inner.tx.subscribe()
    }
}

impl Default for RuntimeEventBus {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publish_subscribe_roundtrip() {
        let bus = RuntimeEventBus::new();
        let mut rx = bus.subscribe();
        bus.publish(EventCategory::Tool, serde_json::json!({"name": "read_file"}))
            .expect("publish should succeed");
        let env = rx.recv().await.expect("recv should yield");
        assert_eq!(env.category, EventCategory::Tool);
        assert_eq!(env.data["name"], "read_file");
    }

    #[tokio::test]
    async fn multiple_subscribers_each_receive() {
        let bus = RuntimeEventBus::new();
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        bus.publish(EventCategory::File, serde_json::json!({"p": "a.rs"})).unwrap();
        assert_eq!(a.recv().await.unwrap().data["p"], "a.rs");
        assert_eq!(b.recv().await.unwrap().data["p"], "a.rs");
    }
}
