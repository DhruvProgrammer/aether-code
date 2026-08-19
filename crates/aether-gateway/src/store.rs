//! Validation store (gateway spec §11, §12).
//!
//! Persists per-role [`ModelRoleSnapshot`]s under `~/.aether/validations.json`.
//! The UI consults `is_valid_for(&current_fingerprint)` before enabling
//! Save/Activate. The file store never contains secrets — only env-var names
//! and fingerprints.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::fingerprint::ModelRoleSnapshot;
use crate::role::Role;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoreFile {
    /// role string → snapshot
    #[serde(default)]
    roles: HashMap<String, ModelRoleSnapshot>,
}

/// Filesystem-backed store of last successful validations.
#[derive(Debug, Clone)]
pub struct ValidationStore {
    path: std::path::PathBuf,
    state: StoreFile,
}

/// A role's validation record as returned to callers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleValidation {
    pub role: Role,
    pub snapshot: Option<ModelRoleSnapshot>,
    /// True when the stored snapshot's fingerprint matches `current`.
    pub valid: bool,
    /// Human reason when invalid: not validated yet / config changed.
    pub reason: Option<String>,
}

impl ValidationStore {
    /// Open (or create) the validation store at `path`.
    pub fn open(path: std::path::PathBuf) -> Result<Self, std::io::Error> {
        let state = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(txt) => serde_json::from_str(&txt).unwrap_or_default(),
                Err(_) => StoreFile::default(),
            }
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            StoreFile::default()
        };
        Ok(Self { path, state })
    }

    /// Default location: `~/.aether/validations.json`.
    pub fn default_path() -> Result<Self, std::io::Error> {
        let home = dirs::home_dir().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "cannot resolve home directory")
        })?;
        Self::open(home.join(".aether").join("validations.json"))
    }

    /// Record a successful validation. Invalidates any previous snapshot for
    /// the same role (replaced wholesale).
    pub fn record(&mut self, snapshot: ModelRoleSnapshot) -> Result<(), std::io::Error> {
        self.state.roles.insert(snapshot.role.as_str().to_string(), snapshot);
        let json = serde_json::to_string_pretty(&self.state)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&self.path, json)?;
        Ok(())
    }

    /// Drop a role's validation (e.g. the user de-configured it).
    pub fn clear(&mut self, role: Role) -> Result<(), std::io::Error> {
        self.state.roles.remove(role.as_str());
        let json = serde_json::to_string_pretty(&self.state)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&self.path, json)?;
        Ok(())
    }

    /// Query validation state for `role` against the binding's *current*
    /// fingerprint. A stored snapshot that no longer matches means the config
    /// changed after validation → Save/Activate must be disabled until the
    /// user re-runs Check API Response.
    pub fn status_for(&self, role: Role, current_fingerprint: &str) -> RoleValidation {
        match self.state.roles.get(role.as_str()) {
            None => RoleValidation {
                role,
                snapshot: None,
                valid: false,
                reason: Some("not validated yet — run Check API Response first".into()),
            },
            Some(snap) => {
                if snap.fingerprint == current_fingerprint {
                    RoleValidation { role, snapshot: Some(snap.clone()), valid: true, reason: None }
                } else {
                    RoleValidation {
                        role,
                        snapshot: Some(snap.clone()),
                        valid: false,
                        reason: Some("configuration changed after validation — re-run Check API Response".into()),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprint::ModelRoleSnapshot;

    fn snap(fp: &str) -> ModelRoleSnapshot {
        ModelRoleSnapshot {
            role: Role::Executor,
            model_key: "exec".into(),
            provider_id: "openai_compatible".into(),
            base_url: "https://x/v1".into(),
            model_id: "gpt".into(),
            api_key_env: "K".into(),
            fingerprint: fp.into(),
            validated_at: "now".into(),
        }
    }

    fn tmp(name: &str) -> ValidationStore {
        let p = std::env::temp_dir().join(format!("gw-valid-{}-{}", name, uuid_like()));
        ValidationStore::open(p).unwrap()
    }

    fn uuid_like() -> String {
        format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())
    }

    #[test]
    fn missing_validation_is_invalid_with_reason() {
        let s = tmp("missing");
        let st = s.status_for(Role::Controller, "fp");
        assert!(!st.valid);
        assert!(st.reason.unwrap().contains("not validated yet"));
    }

    #[test]
    fn record_then_match_is_valid() {
        let mut s = tmp("match");
        s.record(snap("abc")).unwrap();
        let st = s.status_for(Role::Executor, "abc");
        assert!(st.valid);
        assert!(st.reason.is_none());
    }

    #[test]
    fn config_change_invalidates() {
        let mut s = tmp("invalidate");
        s.record(snap("abc")).unwrap();
        let st = s.status_for(Role::Executor, "def");
        assert!(!st.valid);
        assert!(st.reason.unwrap().contains("configuration changed"));
    }

    #[test]
    fn clear_removes_role() {
        let mut s = tmp("clear");
        s.record(snap("abc")).unwrap();
        s.clear(Role::Executor).unwrap();
        let st = s.status_for(Role::Executor, "abc");
        assert!(!st.valid);
    }

    #[test]
    fn persistence_roundtrip() {
        let p = std::env::temp_dir().join(format!("gw-valid-rt-{}", uuid_like()));
        {
            let mut s = ValidationStore::open(p.clone()).unwrap();
            s.record(snap("xyz")).unwrap();
        }
        let s2 = ValidationStore::open(p).unwrap();
        let st = s2.status_for(Role::Executor, "xyz");
        assert!(st.valid);
    }
}
