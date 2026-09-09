//! [`ServiceRegistry`] — the `ctx.<key>` equivalent.
//!
//! A service is a named capability with three roles (Definition /
//! Provider / Consumer). The registry owns the Definition role: it
//! maps a stable `&'static str` key to one type-erased provider value.
//! Exactly one provider is active per key; `replace()` swaps it
//! atomically (the provider-swap seam). Consumers resolve per call
//! via [`ServiceRegistry::get`] and never import an implementation.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ServiceError {
    /// No provider registered under `key`. `waiter` names the plugin
    /// (or subsystem) that asked, so boot audits can report
    /// `pending (waiting for services: …)`.
    #[error("service '{key}' is not registered (waited by '{waiter}')")]
    Missing { key: &'static str, waiter: String },
    /// A second provider tried to claim `key`. Single-active seams
    /// reject this (multi-route seams must use a registry value that
    /// supports multiple routes internally, like the tool service).
    #[error("service '{key}' already provided by '{owner}'; '{contender}' rejected")]
    Duplicate { key: &'static str, owner: String, contender: String },
    /// A value was present but had an unexpected concrete type —
    /// a programming error (wrong `get::<T>`), never a config error.
    #[error("service '{key}' holds an incompatible type for this consumer")]
    TypeMismatch { key: &'static str },
}

struct Entry {
    value: Arc<dyn Any + Send + Sync>,
    owner: String,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The value is type-erased; only the owner is printable.
        f.debug_struct("Entry").field("owner", &self.owner).finish_non_exhaustive()
    }
}

/// Type-erased service map keyed by stable `&'static str` names.
///
/// Owns no threads and performs no I/O; the host wraps it in an
/// `RwLock`. All methods are synchronous.
#[derive(Debug, Default)]
pub struct ServiceRegistry {
    entries: HashMap<&'static str, Entry>,
}

impl ServiceRegistry {
    pub fn new() -> Self {
        Self { entries: HashMap::new() }
    }

    /// Publish a provider. Fails when the key is taken — use
    /// [`replace`](Self::replace) for deliberate swaps.
    pub fn provide<T: Send + Sync + 'static>(
        &mut self,
        key: &'static str,
        value: T,
        owner: impl Into<String>,
    ) -> Result<(), ServiceError> {
        let owner = owner.into();
        if let Some(existing) = self.entries.get(key) {
            return Err(ServiceError::Duplicate {
                key,
                owner: existing.owner.clone(),
                contender: owner,
            });
        }
        self.entries.insert(key, Entry { value: Arc::new(value), owner });
        Ok(())
    }

    /// Atomically swap the provider for `key`, returning whether a
    /// previous provider existed. This is the provider-swap seam:
    /// one call moves an entire capability (local → sandbox →
    /// remote) without touching consumers.
    pub fn replace<T: Send + Sync + 'static>(
        &mut self,
        key: &'static str,
        value: T,
        owner: impl Into<String>,
    ) -> bool {
        let had = self.entries.contains_key(key);
        self.entries.insert(key, Entry { value: Arc::new(value), owner: owner.into() });
        had
    }

    /// Resolve a provider for a consumer. The returned `Arc<T>` keeps
    /// the provider alive past disposal; consumers doing long-lived
    /// work should re-resolve per call instead.
    pub fn get<T: Send + Sync + 'static>(
        &self,
        key: &'static str,
        waiter: impl Into<String>,
    ) -> Result<Arc<T>, ServiceError> {
        let entry = self.entries.get(key).ok_or_else(|| ServiceError::Missing {
            key,
            waiter: waiter.into(),
        })?;
        entry
            .value
            .clone()
            .downcast::<T>()
            .map_err(|_| ServiceError::TypeMismatch { key })
    }

    /// True when a provider is registered under `key`.
    pub fn contains(&self, key: &'static str) -> bool {
        self.entries.contains_key(key)
    }

    /// Remove a provider (used by disposal). Returns the former owner.
    pub fn remove(&mut self, key: &'static str) -> Option<String> {
        self.entries.remove(key).map(|e| e.owner)
    }

    /// Owner names, for boot reports and dumps.
    pub fn owners(&self) -> Vec<(&'static str, &str)> {
        let mut out: Vec<(&'static str, &str)> =
            self.entries.iter().map(|(k, e)| (*k, e.owner.as_str())).collect();
        out.sort_by_key(|(k, _)| *k);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provide_get_roundtrip() {
        let mut reg = ServiceRegistry::new();
        reg.provide("tools", 41u32, "core").unwrap();
        let v: Arc<u32> = reg.get("tools", "test").unwrap();
        assert_eq!(*v, 41u32);
    }

    #[test]
    fn duplicate_names_owner_and_contender() {
        let mut reg = ServiceRegistry::new();
        reg.provide("shell", "local".to_string(), "bash-local").unwrap();
        let err = reg.provide("shell", "sandbox".to_string(), "bash-sandbox").unwrap_err();
        assert_eq!(
            err,
            ServiceError::Duplicate {
                key: "shell",
                owner: "bash-local".into(),
                contender: "bash-sandbox".into()
            }
        );
        // Original provider is untouched.
        let v: Arc<String> = reg.get("shell", "test").unwrap();
        assert_eq!(v.as_str(), "local");
    }

    #[test]
    fn replace_swaps_atomically_and_reports() {
        let mut reg = ServiceRegistry::new();
        assert!(!reg.replace("shell", "local".to_string(), "bash-local"));
        assert!(reg.replace("shell", "sandbox".to_string(), "bash-sandbox"));
        let v: Arc<String> = reg.get("shell", "test").unwrap();
        assert_eq!(v.as_str(), "sandbox");
    }

    #[test]
    fn missing_names_key_and_waiter() {
        let reg = ServiceRegistry::new();
        let err = reg.get::<u32>("llm", "tool-bash").unwrap_err();
        assert_eq!(
            err,
            ServiceError::Missing { key: "llm", waiter: "tool-bash".into() }
        );
    }

    #[test]
    fn type_mismatch_is_a_programming_error() {
        let mut reg = ServiceRegistry::new();
        reg.provide("tools", 1u32, "core").unwrap();
        let err = reg.get::<String>("tools", "test").unwrap_err();
        assert_eq!(err, ServiceError::TypeMismatch { key: "tools" });
    }
}
