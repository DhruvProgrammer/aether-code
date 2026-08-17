//! Auditable decision log + event sink.

use super::policy::Permission;
use super::rule::RuleMatch;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub timestamp: DateTime<Utc>,
    pub agent_id: Option<String>,
    pub tool: Option<String>,
    pub operation: String,
    pub target: String,
    pub verdict: Permission,
    pub matched_rule: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionVerdict {
    Allowed,
    Denied,
    Asked,
}

impl DecisionVerdict {
    pub fn from(p: Permission) -> Self {
        match p {
            Permission::Allow => Self::Allowed,
            Permission::Deny => Self::Denied,
            Permission::Ask => Self::Asked,
        }
    }
}

#[derive(Debug, Default)]
pub struct DecisionLog {
    records: Mutex<Vec<DecisionRecord>>,
}

impl DecisionLog {
    pub fn new() -> Self { Self::default() }
    pub fn record(&self, rec: DecisionRecord) {
        self.records.lock().unwrap().push(rec);
    }
    pub fn snapshot(&self) -> Vec<DecisionRecord> {
        self.records.lock().unwrap().clone()
    }
    pub fn len(&self) -> usize { self.records.lock().unwrap().len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
    pub fn clear(&self) { self.records.lock().unwrap().clear(); }
}

/// Anything that wants to hear about permission decisions.
pub trait PermissionEventSink: Send + Sync {
    fn on_decision(&self, rec: &DecisionRecord);
}

/// In-memory sink for tests + the default.
#[derive(Default, Clone)]
pub struct InMemorySink {
    pub records: Arc<Mutex<Vec<DecisionRecord>>>,
}
impl PermissionEventSink for InMemorySink {
    fn on_decision(&self, rec: &DecisionRecord) {
        self.records.lock().unwrap().push(rec.clone());
    }
}

/// Sink that does nothing.
#[derive(Default, Clone)]
pub struct NullSink;
impl PermissionEventSink for NullSink {
    fn on_decision(&self, _rec: &DecisionRecord) {}
}

/// Event payload sent over the bus (serializable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionEvent {
    pub agent_id: Option<String>,
    pub tool: Option<String>,
    pub operation: String,
    pub target: String,
    pub verdict: Permission,
    pub matched_rule: Option<String>,
    pub reason: Option<String>,
    pub timestamp: DateTime<Utc>,
}

impl From<&DecisionRecord> for PermissionEvent {
    fn from(r: &DecisionRecord) -> Self {
        Self {
            agent_id: r.agent_id.clone(),
            tool: r.tool.clone(),
            operation: r.operation.clone(),
            target: r.target.clone(),
            verdict: r.verdict,
            matched_rule: r.matched_rule.clone(),
            reason: r.reason.clone(),
            timestamp: r.timestamp,
        }
    }
}

impl RuleMatch {
    pub fn summary(&self) -> String {
        format!("{} on {}", self.rule.operation.label(), self.rule.scope.label())
    }
}

impl super::scope::ResourceScope {
    pub fn label(&self) -> String {
        match self {
            Self::Any => "*".into(),
            Self::Path { value } | Self::Glob { value } | Self::Host { value }
            | Self::CommandSubstring { value } | Self::Tool { value } | Self::Provider { value }
            | Self::Model { value } | Self::Mcp { value } | Self::EnvVar { value }
            | Self::Secret { value } => value.clone(),
        }
    }
}
