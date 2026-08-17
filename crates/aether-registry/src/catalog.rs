//! Model descriptors.

use crate::capabilities::CapabilityMatrix;
use serde::{Deserialize, Serialize};

/// Status of a known model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    /// Confirmed reachable via a recent health check.
    Available,
    /// Health check failed — model is registered but flagged unreliable.
    Degraded,
    /// Provider reported this model as removed / EOL.
    Deprecated,
    /// Health check never run.
    Unknown,
}

/// One model in the catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDescriptor {
    pub id: String,
    pub display_name: String,
    pub provider_id: String,
    pub capabilities: CapabilityMatrix,
    pub status: ModelStatus,
    /// Optional human-readable notes.
    pub notes: Option<String>,
}

impl ModelDescriptor {
    pub fn new(id: impl Into<String>, provider_id: impl Into<String>, caps: CapabilityMatrix) -> Self {
        Self {
            id: id.into(),
            display_name: String::new(),
            provider_id: provider_id.into(),
            capabilities: caps,
            status: ModelStatus::Unknown,
            notes: None,
        }
    }

    pub fn with_display_name(mut self, n: impl Into<String>) -> Self { self.display_name = n.into(); self }
    pub fn with_status(mut self, s: ModelStatus) -> Self { self.status = s; self }
    pub fn with_notes(mut self, n: impl Into<String>) -> Self { self.notes = Some(n.into()); self }
}
