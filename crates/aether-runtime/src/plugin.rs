//! [`Plugin`] — the unit of composition.
//!
//! A plugin is a metadata record plus an async `apply()` that
//! contributes services, tools, listeners and guards to the host as
//! reversible effects. Plugins declare `inject()` service
//! dependencies; the host refuses to boot when a dependency is
//! missing and says exactly which plugin waits for which service
//! (the `pending (waiting for services: …)` audit).

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use crate::effects::EffectStack;
use crate::host::HostInner;
use crate::scope::ScopeKey;

/// Static plugin metadata. `id` is the stable composition-row target
/// (patches address rows by id); `name` is the human label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInfo {
    /// Stable id, e.g. `"tools-fs"`. Lowercase alphanumeric + dashes.
    pub id: String,
    /// Human label, e.g. `"Filesystem tools"`.
    pub name: String,
    /// Semver string, informational.
    pub version: String,
    /// One-line description.
    pub description: String,
}

impl PluginInfo {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            version: version.into(),
            description: description.into(),
        }
    }
}

/// The context a plugin applies into. Registrations made through it
/// are effects: push each disposer onto [`effects`](Self::effects) so
/// host shutdown (or row replacement) unwinds them.
pub struct PluginContext {
    /// Shared host interior (services, bus, tool service, warnings).
    pub host: Arc<HostInner>,
    /// Scope the plugin applies into (global for boot rows).
    pub scope: ScopeKey,
    /// Effect stack; the host drains it on shutdown in reverse
    /// activation order.
    pub effects: EffectStack,
    /// The row's `config` value (whole-row replace semantics mean this
    /// is always the complete config, never a merge).
    pub config: Value,
    /// Non-fatal notes surfaced in the boot report (unknown config
    /// keys, deprecations). Fatal problems must be `Err`.
    pub warnings: Vec<String>,
}

impl PluginContext {
    /// Record a non-fatal note for the boot report.
    pub fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }

    /// Warn about config keys the plugin does not understand.
    /// Plugins with a fixed key set call this to keep typos audible.
    pub fn warn_unknown_keys(&mut self, known: &[&str]) {
        if let Value::Object(map) = &self.config {
            for key in map.keys() {
                if !known.contains(&key.as_str()) {
                    self.warnings.push(format!("unknown config key '{key}' (ignored)"));
                }
            }
        }
    }
}

impl std::fmt::Debug for PluginContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginContext")
            .field("scope", &self.scope)
            .field("config", &self.config)
            .field("warnings", &self.warnings)
            .finish_non_exhaustive()
    }
}

/// A unit of composition.
///
/// Implementors are usually small structs holding whatever the plugin
/// needs to build its contributions (tool lists, server addresses,
/// store handles). `apply` must be idempotent with respect to its own
/// effects only in the sense that every registration pushes a
/// disposer — the host never applies the same row twice.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// Static metadata.
    fn info(&self) -> PluginInfo;

    /// Service keys that must exist before this plugin applies
    /// (the `inject` gate). Defaults to none. A missing service
    /// fails boot with a pending-service audit naming this plugin.
    fn inject(&self) -> Vec<String> {
        Vec::new()
    }

    /// Contribute to the host. Push every registration's disposer
    /// onto `ctx.effects`. Return `Err` to fail boot loudly (the host
    /// unwinds already-activated rows first).
    async fn apply(&self, ctx: &mut PluginContext) -> anyhow::Result<()>;
}
