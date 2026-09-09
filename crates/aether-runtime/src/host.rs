//! [`PluginHost`] — boot with dependency-gated activation and
//! reverse-order shutdown.
//!
//! Boot is fail-loud by design: a row whose factory is unknown, a
//! row whose `inject` services are absent, or a plugin whose `apply`
//! errors aborts the whole boot with an audit that names the row and
//! the missing services (`pending (waiting for services: …)`).
//! Partial tool sets are worse than errors, so there is no silent
//! fallback — already-activated rows unwind first.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use thiserror::Error;
use tokio::sync::RwLock;

use crate::catalog;
use crate::compose::{ComposedRow, Composition, Origin, PluginRegistry};
use crate::effects::EffectStack;
use crate::events::Bus;
use crate::plugin::{PluginContext, PluginInfo};
use crate::scope::{ScopeChain, ScopeKey};
use crate::services::ServiceRegistry;
use crate::tools::ToolService;

#[derive(Debug, Error)]
pub enum BootError {
    /// No factory registered for the row's plugin name.
    #[error("row '{row}' names unknown plugin '{plugin}' (not in the factory registry)")]
    UnknownPlugin { row: String, plugin: String },
    /// One or more rows wait for services that never appeared.
    #[error("boot blocked: {0}")]
    PendingServices(String),
    /// A plugin's `apply` failed. Already-activated rows unwound first.
    #[error("plugin '{row}' failed to apply: {source}")]
    ApplyFailed { row: String, #[source] source: anyhow::Error },
}

impl BootError {
    fn pending(details: Vec<(String, Vec<String>)>) -> Self {
        let lines: Vec<String> = details
            .into_iter()
            .map(|(row, missing)| {
                format!("  - '{row}' pending (waiting for services: {})", missing.join(", "))
            })
            .collect();
        Self::PendingServices(format!("\n{}", lines.join("\n")))
    }
}

/// One activated row (for reports and ordered shutdown).
#[derive(Debug)]
pub struct Activation {
    pub row_id: String,
    pub plugin: PluginInfo,
    pub origin: Origin,
    pub replaced: bool,
    pub warnings: Vec<String>,
}

/// What boot did, for logs and `--debug`.
#[derive(Debug, Default)]
pub struct BootReport {
    pub activated: Vec<String>,
    pub skipped: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

/// Shared host interior. `PluginContext` carries an `Arc` of this so
/// plugins resolve services, dispatch events and register tools.
pub struct HostInner {
    /// Service repository (`provide` / `get` / `replace`).
    pub services: RwLock<ServiceRegistry>,
    /// Mode-tagged event bus.
    pub bus: Bus,
    /// The tool seam.
    pub tools: ToolService,
}

impl std::fmt::Debug for HostInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostInner").finish_non_exhaustive()
    }
}

/// The plugin runtime. Owns services, bus, tool seam, scopes,
/// activations and the factory registry.
pub struct PluginHost {
    inner: Arc<HostInner>,
    factories: Mutex<PluginRegistry>,
    activations: Mutex<Vec<Activation>>,
    effect_stacks: Mutex<Vec<EffectStack>>,
    next_scope: AtomicU64,
    booted: Mutex<bool>,
}

impl std::fmt::Debug for PluginHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginHost").finish_non_exhaustive()
    }
}

impl PluginHost {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(HostInner {
                services: RwLock::new(ServiceRegistry::new()),
                bus: Bus::new(),
                tools: ToolService::new(),
            }),
            factories: Mutex::new(PluginRegistry::new()),
            activations: Mutex::new(Vec::new()),
            effect_stacks: Mutex::new(Vec::new()),
            next_scope: AtomicU64::new(1),
            booted: Mutex::new(false),
        }
    }

    /// Shared interior for plugin contexts and consumers.
    pub fn inner(&self) -> &Arc<HostInner> {
        &self.inner
    }

    /// Register a plugin factory (`PluginRegistry::register`).
    pub fn register_factory(
        &self,
        name: impl Into<String>,
        factory: crate::compose::Factory,
    ) {
        self.factories
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .register(name, factory);
    }

    /// Mint a fresh session scope key.
    pub fn mint_scope(&self) -> ScopeKey {
        ScopeKey(self.next_scope.fetch_add(1, Ordering::Relaxed))
    }

    /// Boot the composed rows. Dependency-gated with multi-pass
    /// resolution: rows apply in composition order, but a row whose
    /// services are not ready yet defers to a later pass instead of
    /// failing — activation order never depends on row order, only
    /// composition order among ready rows is row order (deterministic).
    /// After a pass with no progress and rows still deferred, boot
    /// fails loudly with a pending-service audit. Rows with skip
    /// reasons are recorded, never activated. Double boot is an error
    /// (build a fresh host instead — hosts are cheap).
    pub async fn boot(&self, composition: Composition) -> Result<BootReport, BootError> {
        {
            let mut booted = self.booted.lock().unwrap_or_else(|e| e.into_inner());
            if *booted {
                return Err(BootError::PendingServices("host already booted".into()));
            }
            *booted = true;
        }
        let mut report = BootReport::default();
        report.warnings.extend(composition.warnings);

        // Pass 0: skips (intent is order-independent).
        let mut deferred: Vec<&ComposedRow> = Vec::new();
        for composed in &composition.rows {
            match composed.row.skip_reason() {
                Some(reason) => report.skipped.push((composed.row.id.clone(), reason)),
                None => deferred.push(composed),
            }
        }

        // Passes 1..n: activate whatever is ready; defer the rest.
        while !deferred.is_empty() {
            let mut still: Vec<&ComposedRow> = Vec::new();
            let mut missing_now: Vec<(String, Vec<String>)> = Vec::new();
            let mut progressed = false;
            for composed in deferred {
                let row = &composed.row;
                let plugin = {
                    let factories =
                        self.factories.lock().unwrap_or_else(|e| e.into_inner());
                    match factories.create(&row.plugin) {
                        Some(p) => p,
                        None => {
                            self.unwind_partial().await;
                            return Err(BootError::UnknownPlugin {
                                row: row.id.clone(),
                                plugin: row.plugin.clone(),
                            });
                        }
                    }
                };
                // Effective inject: row override wins, else the plugin's.
                let mut wants = if row.inject.is_empty() {
                    plugin.inject()
                } else {
                    row.inject.clone()
                };
                wants.sort();
                wants.dedup();
                let missing: Vec<String> = {
                    let services = self.inner.services.read().await;
                    wants.into_iter().filter(|k| !service_present(&services, k)).collect()
                };
                if !missing.is_empty() {
                    missing_now.push((row.id.clone(), missing));
                    still.push(composed);
                    continue;
                }
                let mut ctx = PluginContext {
                    host: self.inner.clone(),
                    scope: ScopeKey::GLOBAL,
                    effects: EffectStack::new(),
                    config: row.config.clone(),
                    warnings: Vec::new(),
                };
                if let Err(source) = plugin.apply(&mut ctx).await {
                    self.unwind_partial().await;
                    return Err(BootError::ApplyFailed { row: row.id.clone(), source });
                }
                let info = plugin.info();
                report.warnings.extend(
                    ctx.warnings.iter().map(|w| format!("{}: {w}", row.id)),
                );
                let warnings = ctx.warnings.clone();
                self.effect_stacks.lock().unwrap_or_else(|e| e.into_inner()).push(ctx.effects);
                self.activations.lock().unwrap_or_else(|e| e.into_inner()).push(Activation {
                    row_id: row.id.clone(),
                    plugin: info,
                    origin: composed.origin,
                    replaced: composed.replaced,
                    warnings,
                });
                report.activated.push(row.id.clone());
                progressed = true;
            }
            if still.is_empty() {
                break;
            }
            if !progressed {
                self.unwind_partial().await;
                return Err(BootError::pending(missing_now));
            }
            deferred = still;
        }

        // host/booted is unfiltered: shell-owned observers always see it.
        let def = catalog::lookup("host/booted").expect("catalog has host/booted");
        self.inner
            .bus
            .dispatch(
                def,
                &ScopeChain::for_scope(ScopeKey::GLOBAL),
                serde_json::json!({
                    "rows": report.activated,
                    "skipped": report.skipped.iter().map(|(id, reason)| {
                        serde_json::json!({"id": id, "reason": reason})
                    }).collect::<Vec<Value>>(),
                }),
            )
            .await;
        Ok(report)
    }

    /// Unwind already-activated rows in reverse activation order
    /// (used when boot fails partway — no partial tool set survives).
    async fn unwind_partial(&self) {
        let mut stacks = self.effect_stacks.lock().unwrap_or_else(|e| e.into_inner());
        while let Some(mut stack) = stacks.pop() {
            stack.unwind();
        }
        drop(stacks);
        self.activations.lock().unwrap_or_else(|e| e.into_inner()).clear();
        *self.booted.lock().unwrap_or_else(|e| e.into_inner()) = false;
    }

    /// Shut down: unwind every activation's effects in reverse
    /// activation order, then emit `host/shutdown` with the unwind
    /// record. Idempotent.
    pub async fn shutdown(&self) -> Vec<String> {
        let mut order = Vec::new();
        {
            let mut stacks = self.effect_stacks.lock().unwrap_or_else(|e| e.into_inner());
            while let Some(mut stack) = stacks.pop() {
                order.extend(stack.unwind());
            }
            self.activations.lock().unwrap_or_else(|e| e.into_inner()).clear();
            *self.booted.lock().unwrap_or_else(|e| e.into_inner()) = false;
        }
        let def = catalog::lookup("host/shutdown").expect("catalog has host/shutdown");
        self.inner
            .bus
            .dispatch(
                def,
                &ScopeChain::for_scope(ScopeKey::GLOBAL),
                serde_json::json!({ "unwound": order }),
            )
            .await;
        order
    }

    /// Activated rows, in activation order.
    pub fn activations(&self) -> Vec<(String, Origin)> {
        self.activations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|a| (a.row_id.clone(), a.origin))
            .collect()
    }

    /// Total live (un-unwound) effects across activations.
    pub fn live_effect_count(&self) -> usize {
        self.effect_stacks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|s| s.live_labels().len())
            .sum()
    }
}

impl Default for PluginHost {
    fn default() -> Self {
        Self::new()
    }
}

fn service_present(services: &ServiceRegistry, key: &str) -> bool {
    // Service keys are `&'static str`; row-declared gates are
    // dynamic strings. Match by name. (Keys are leaked only when a
    // plugin needs a fully dynamic key — none do today; see
    // ServiceRegistry docs.)
    services.owners().iter().any(|(k, _)| *k == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::{Row, compose};
    use crate::plugin::{Plugin, PluginInfo};
    use async_trait::async_trait;
    use std::sync::Arc;

    struct ProbePlugin {
        id: &'static str,
        inject: Vec<String>,
        service: Option<(&'static str, String)>,
        fail: bool,
        log: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Plugin for ProbePlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo::new(self.id, self.id, "0.0.0", "probe")
        }

        fn inject(&self) -> Vec<String> {
            self.inject.clone()
        }

        async fn apply(&self, ctx: &mut PluginContext) -> anyhow::Result<()> {
            if self.fail {
                anyhow::bail!("probe failure");
            }
            self.log.lock().unwrap().push(self.id.to_string());
            if let Some((key, value)) = &self.service {
                ctx.host.services.write().await.provide(key, value.clone(), self.id)?;
            }
            let log = self.log.clone();
            let id = self.id;
            ctx.effects.push(crate::effects::Effect::new(format!("probe:{id}"), move || {
                log.lock().unwrap().push(format!("down:{id}"));
            }));
            Ok(())
        }
    }

    fn host_with(log: &Arc<Mutex<Vec<String>>>) -> Arc<PluginHost> {
        let host = Arc::new(PluginHost::new());
        let rows: [(&str, &'static str, Vec<String>, Option<(&'static str, String)>, bool); 4] = [
            ("p:a", "a", vec![], None, false),
            ("p:b", "b", vec!["svc-a".to_string()], None, false),
            ("p:provider", "provider", vec![], Some(("svc-a", "v".to_string())), false),
            ("p:fail", "fail", vec![], None, true),
        ];
        for (name, id, inject, service, fail) in rows {
            let log = log.clone();
            host.register_factory(
                name,
                Box::new(move || {
                    Arc::new(ProbePlugin {
                        id,
                        inject: inject.clone(),
                        service: service.clone(),
                        fail,
                        log: log.clone(),
                    }) as Arc<dyn Plugin>
                }),
            );
        }
        host
    }

    fn row(id: &str, plugin: &str) -> Row {
        Row::new(id, plugin)
    }

    #[tokio::test]
    async fn activation_is_order_independent() {
        // `b` needs `svc-a`, provided by `provider` listed LAST.
        // Multi-pass resolution activates providers first even so.
        for rows in [
            vec![row("b", "p:b"), row("a", "p:a"), row("provider", "p:provider")],
            vec![row("provider", "p:provider"), row("b", "p:b"), row("a", "p:a")],
        ] {
            let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
            let host = host_with(&log);
            let report = host.boot(compose(rows, vec![])).await.unwrap();
            assert_eq!(report.activated.len(), 3);
            let log = log.lock().unwrap().clone();
            let pos = |id: &str| log.iter().position(|l| l == id).unwrap();
            // The provider always applies before its dependent.
            assert!(pos("provider") < pos("b"), "order-independent: {log:?}");
            host.shutdown().await;
        }
    }

    #[tokio::test]
    async fn pending_services_fail_loud_with_audit() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let host = host_with(&log);
        // `b` before its provider: pending audit names row + service.
        let c = compose(vec![row("b", "p:b"), row("a", "p:a")], vec![]);
        let err = host.boot(c).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("'b'"), "audit names the row: {msg}");
        assert!(msg.contains("svc-a"), "audit names the missing service: {msg}");
        assert!(msg.contains("pending"), "audit uses pending language: {msg}");
    }

    #[tokio::test]
    async fn happy_path_boots_and_shuts_down_in_reverse() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let host = host_with(&log);
        let c = compose(
            vec![row("provider", "p:provider"), row("a", "p:a"), row("b", "p:b")],
            vec![],
        );
        let report = host.boot(c).await.unwrap();
        assert_eq!(report.activated, vec!["provider", "a", "b"]);
        assert_eq!(host.live_effect_count(), 3);
        let order = host.shutdown().await;
        assert_eq!(order, vec!["probe:b", "probe:a", "probe:provider"]);
        let log = log.lock().unwrap().clone();
        assert_eq!(log, vec!["provider", "a", "b", "down:b", "down:a", "down:provider"]);
        assert_eq!(host.live_effect_count(), 0);
    }

    #[tokio::test]
    async fn unknown_plugin_fails_loud() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let host = host_with(&log);
        let c = compose(vec![row("x", "p:missing")], vec![]);
        let err = host.boot(c).await.unwrap_err();
        assert!(err.to_string().contains("p:missing"));
    }

    #[tokio::test]
    async fn apply_failure_unwinds_and_reports() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let host = host_with(&log);
        let c = compose(vec![row("a", "p:a"), row("f", "p:fail")], vec![]);
        let err = host.boot(c).await.unwrap_err();
        assert!(err.to_string().contains("'f'"));
    }
}
