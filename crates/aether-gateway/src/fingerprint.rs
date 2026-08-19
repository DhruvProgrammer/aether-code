//! Configuration fingerprinting (gateway spec §11).
//!
//! Every successful validation produces a fingerprint of the configuration
//! that was actually validated. If any relevant field changes afterwards the
//! stored fingerprint no longer matches and re-validation is required. The
//! raw API key is **never** part of the fingerprint — only the env-var
//! *name* that references it.

use serde::{Deserialize, Serialize};

use crate::role::Role;

/// Deterministic 64-bit FNV-1a rendered as hex. Stable across runs/platforms;
/// sufficient for configuration identity (this is not a security boundary).
pub fn sha256_hex(input: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in input.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}

/// Compute the fingerprint of a role binding + the provider configuration it
/// resolves to. Fields that influence behaviour are included; the API key
/// itself is excluded (only `api_key_env`, the variable name).
pub fn fingerprint_binding(
    role: Role,
    provider_id: &str,
    base_url: &str,
    model_id: &str,
    api_key_env: &str,
    extra_body: Option<&serde_json::Value>,
) -> String {
    let canon_extra = extra_body
        .map(|v| serde_json::to_string(v).unwrap_or_default())
        .unwrap_or_default();
    // Pipe-separated canonical string. Trailing whitespace is trimmed from
    // user-supplied URLs/models so superficial edits don't chum the hash
    // while real changes always do.
    let canonical = format!(
        "{}|{}|{}|{}|{}|{}",
        role.as_str(),
        provider_id.trim(),
        base_url.trim(),
        model_id.trim(),
        api_key_env.trim(),
        canon_extra,
    );
    sha256_hex(&canonical)
}

/// Snapshot of what was validated for one role, persisted next to the config.
/// Never contains secret material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRoleSnapshot {
    pub role: Role,
    /// Key into the `[models]` map.
    pub model_key: String,
    /// Adapter id, e.g. `openai_compatible`.
    pub provider_id: String,
    pub base_url: String,
    pub model_id: String,
    /// Environment variable name holding the key (not the key itself).
    pub api_key_env: String,
    pub fingerprint: String,
    /// RFC3339 timestamp of the successful validation.
    pub validated_at: String,
}

impl ModelRoleSnapshot {
    pub fn matches(&self, other_fingerprint: &str) -> bool {
        self.fingerprint == other_fingerprint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable() {
        let a = fingerprint_binding(Role::Executor, "openai_compatible", "https://x/v1", "gpt", "KEY", None);
        let b = fingerprint_binding(Role::Executor, "openai_compatible", "https://x/v1", "gpt", "KEY", None);
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn fingerprint_changes_with_relevant_fields() {
        let base = fingerprint_binding(Role::Executor, "openai_compatible", "https://x/v1", "gpt", "KEY", None);
        let diff_url = fingerprint_binding(Role::Executor, "openai_compatible", "https://y/v1", "gpt", "KEY", None);
        let diff_model = fingerprint_binding(Role::Executor, "openai_compatible", "https://x/v1", "other", "KEY", None);
        let diff_provider = fingerprint_binding(Role::Executor, "custom", "https://x/v1", "gpt", "KEY", None);
        let diff_key_env = fingerprint_binding(Role::Executor, "openai_compatible", "https://x/v1", "gpt", "OTHER", None);
        let diff_role = fingerprint_binding(Role::Controller, "openai_compatible", "https://x/v1", "gpt", "KEY", None);
        assert_ne!(base, diff_url);
        assert_ne!(base, diff_model);
        assert_ne!(base, diff_provider);
        assert_ne!(base, diff_key_env);
        assert_ne!(base, diff_role);
    }

    #[test]
    fn fingerprint_normalizes_whitespace() {
        let a = fingerprint_binding(Role::Executor, "openai_compatible", "https://x/v1 ", "gpt ", "KEY", None);
        let b = fingerprint_binding(Role::Executor, "openai_compatible", " https://x/v1", "gpt", "KEY", None);
        assert_eq!(a, b);
    }

    #[test]
    fn extra_body_participates() {
        let eb = serde_json::json!({"api_version": "2024-02"});
        let a = fingerprint_binding(Role::Executor, "openai_compatible", "https://x/v1", "gpt", "KEY", None);
        let b = fingerprint_binding(Role::Executor, "openai_compatible", "https://x/v1", "gpt", "KEY", Some(&eb));
        assert_ne!(a, b);
    }

    #[test]
    fn snapshot_matches() {
        let s = ModelRoleSnapshot {
            role: Role::Executor,
            model_key: "exec".into(),
            provider_id: "openai_compatible".into(),
            base_url: "https://x/v1".into(),
            model_id: "gpt".into(),
            api_key_env: "KEY".into(),
            fingerprint: "abc".into(),
            validated_at: "2026-01-01T00:00:00Z".into(),
        };
        assert!(s.matches("abc"));
        assert!(!s.matches("xyz"));
    }
}
