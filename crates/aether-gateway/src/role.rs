//! Explicit per-role model bindings (gateway spec §3, §17).
//!
//! Each of AETHER's three LLM roles carries its **own** provider + model
//! binding. There is no global provider and no dynamic selection: whatever is
//! written here is exactly what the gateway calls.

use serde::{Deserialize, Serialize};

/// The three AETHER model roles. This enum is finite and fixed by design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Model 1 — Big Executor. Primary coding/implementation model. Required.
    Executor,
    /// Model 2 — Small Controller. Planner/orchestrator. Required.
    Controller,
    /// Model 3 — Visual Frontend Reviewer. Optional multimodal reviewer.
    Reviewer,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Executor => "executor",
            Role::Controller => "controller",
            Role::Reviewer => "reviewer",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Role::Executor => "Model 1 — Big Executor",
            Role::Controller => "Model 2 — Small Controller",
            Role::Reviewer => "Model 3 — Visual Frontend Reviewer",
        }
    }

    /// Controller decides *what* happens, never *which* model runs. The
    /// executor always executes implementation.
    pub fn is_required(&self) -> bool {
        matches!(self, Role::Executor | Role::Controller)
    }
}

/// One explicit provider + model binding for a role.
///
/// `model_key` refers to an entry in the `[models]` map of config. The gateway
/// resolves it to a concrete adapter + endpoint at construction. Keeping the
/// key (not inlining base_url/api_key) means the binding never duplicates
/// credential material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleBinding {
    pub role: Role,
    /// Key into the `[models]` config map.
    pub model_key: String,
    /// Whether this binding is active. Only meaningful for optional roles
    /// (Reviewer); required roles being disabled is a configuration error.
    pub enabled: bool,
}

impl RoleBinding {
    pub fn new(role: Role, model_key: impl Into<String>) -> Self {
        Self { role, model_key: model_key.into(), enabled: true }
    }

    pub fn disabled(role: Role, model_key: impl Into<String>) -> Self {
        Self { role, model_key: model_key.into(), enabled: false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_strings_are_stable() {
        assert_eq!(Role::Executor.as_str(), "executor");
        assert_eq!(Role::Controller.as_str(), "controller");
        assert_eq!(Role::Reviewer.as_str(), "reviewer");
    }

    #[test]
    fn required_roles() {
        assert!(Role::Executor.is_required());
        assert!(Role::Controller.is_required());
        assert!(!Role::Reviewer.is_required());
    }

    #[test]
    fn binding_roundtrip() {
        let b = RoleBinding::new(Role::Executor, "nvidia-a");
        let j = serde_json::to_string(&b).unwrap();
        let back: RoleBinding = serde_json::from_str(&j).unwrap();
        assert_eq!(back, b);
    }

    #[test]
    fn disabled_binding() {
        let b = RoleBinding::disabled(Role::Reviewer, "tok-router-c");
        assert!(!b.enabled);
    }
}
