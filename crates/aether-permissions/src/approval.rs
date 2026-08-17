//! Approval channel — when a decision is `Ask`, the engine requests an answer.
//!
//! Implementations include:
//!   * [`PromptChannel`] — read y/N from stdin (TTY).
//!   * [`CallbackChannel`] — synchronous user-supplied closure.
//!   * [`DenyAllChannel`] / [`AllowAllChannel`] — for non-interactive runs.

use super::policy::Permission;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub agent_id: Option<String>,
    pub tool: Option<String>,
    pub operation: String,
    pub target: String,
    pub reason: Option<String>,
    pub risk: Risk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    Once,
    Session,
    Project,
    Always,
}

#[derive(Debug, Clone)]
pub struct ApprovalResponse {
    pub permission: Permission,
    pub scope: ApprovalScope,
}

/// Synchronous channel: callers block until the closure returns.
pub type ApprovalFn = Arc<dyn Fn(&ApprovalRequest) -> ApprovalResponse + Send + Sync>;

#[derive(Clone)]
pub struct CallbackChannel(pub ApprovalFn);

impl ApprovalChannel for CallbackChannel {
    fn request(&self, req: &ApprovalRequest) -> ApprovalResponse {
        (self.0)(req)
    }
}

/// Anything that knows how to resolve an approval.
pub trait ApprovalChannel: Send + Sync {
    fn request(&self, req: &ApprovalRequest) -> ApprovalResponse;
}

/// Non-interactive policies.
#[derive(Default, Clone)]
pub struct DenyAllChannel;
impl ApprovalChannel for DenyAllChannel {
    fn request(&self, _req: &ApprovalRequest) -> ApprovalResponse {
        ApprovalResponse { permission: Permission::Deny, scope: ApprovalScope::Once }
    }
}

#[derive(Default, Clone)]
pub struct AllowAllChannel;
impl ApprovalChannel for AllowAllChannel {
    fn request(&self, _req: &ApprovalRequest) -> ApprovalResponse {
        ApprovalResponse { permission: Permission::Allow, scope: ApprovalScope::Once }
    }
}

/// TTY prompt.
pub struct PromptChannel;
impl ApprovalChannel for PromptChannel {
    fn request(&self, req: &ApprovalRequest) -> ApprovalResponse {
        use std::io::{self, Write};
        let target = &req.target;
        let op = &req.operation;
        eprint!("\n[aether/permission] Allow {op} on {target}? [y/N] (s=session, p=project, a=always-deny): ");
        let _ = io::stderr().flush();
        let mut line = String::new();
        if io::stdin().read_line(&mut line).is_err() {
            return ApprovalResponse { permission: Permission::Deny, scope: ApprovalScope::Once };
        }
        let s = line.trim().to_lowercase();
        match s.as_str() {
            "y" | "yes" => ApprovalResponse { permission: Permission::Allow, scope: ApprovalScope::Once },
            "s" => ApprovalResponse { permission: Permission::Allow, scope: ApprovalScope::Session },
            "p" => ApprovalResponse { permission: Permission::Allow, scope: ApprovalScope::Project },
            "a" | "always-deny" | "n" | "" => ApprovalResponse { permission: Permission::Deny, scope: ApprovalScope::Once },
            _ => ApprovalResponse { permission: Permission::Deny, scope: ApprovalScope::Once },
        }
    }
}
