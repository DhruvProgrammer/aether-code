//! Permission rule + its source.

use super::policy::Permission;
use super::scope::{Operation, ResourceScope};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub operation: Operation,
    pub scope: ResourceScope,
    pub permission: Permission,
    /// Optional human-readable reason (for the approval UI).
    pub reason: Option<String>,
}

impl Rule {
    pub fn new(operation: Operation, scope: ResourceScope, permission: Permission) -> Self {
        Self { operation, scope, permission, reason: None }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn matches(&self, op: Operation, scope: &ResourceScope) -> bool {
        self.operation == op && self.scope.matches(scope)
    }
}

/// Where a rule came from. Used for the audit log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleSource {
    Global,
    Project,
    Role,
    Agent,
    Tool,
    Inline,
}

/// Result of matching a request against the rule set.
#[derive(Debug, Clone)]
pub struct RuleMatch {
    pub rule: Rule,
    pub source: RuleSource,
}
