//! AETHER plugin runtime — the "everything is a plugin" layer.
//!
//! Ported mechanics (not code) from DeepSeek Harness's Cordis
//! architecture — see
//! `reference_architecture/deepseek-harness/REFERENCE_MANIFEST.md`.
//! AETHER-native and dependency-light: this crate depends only on
//! crates already in the workspace lockfile.
//!
//! Concepts:
//!
//! * [`Plugin`] — a unit of composition: static [`PluginInfo`]
//!   (id, version), `inject()` service dependencies, and an async
//!   `apply()` that contributes services, tools, listeners and guards
//!   as reversible [`effects`](EffectStack).
//! * [`ServiceRegistry`] — the `ctx.<key>` equivalent: type-erased
//!   services keyed by `&'static str`, with duplicate-provider
//!   detection and atomic `replace()`.
//! * [`Bus`] — mode-tagged event dispatch (`Emit`, `Waterfall`,
//!   `Serial`, `Parallel`) with scope filtering. Waterfall listeners
//!   receive a [`Next`] continuation: returning without delegating
//!   short-circuits the chain (policy owns the decision).
//! * [`ScopeKey`] — opaque scope identity (the global scope plus one
//!   minted key per session). Registries shadow global contributions
//!   with scope-local ones; dispatch admits untagged listeners plus
//!   the dispatch scope and its ancestors.
//! * [`ToolService`] — the tool seam (Definition role): contributor
//!   registration with disposers, per-scope `restrict{allow,deny}`
//!   overlays, monotonic deny-only [`guards`](ToolService::guard),
//!   and the `tools/pre-execute` → `tools/post-execute` →
//!   `tools/result` interception pipeline.
//! * [`compose`] — profile → patch-layer composition over TOML rows
//!   with whole-row replace semantics, provenance tracking and a
//!   `dump()` preview (the `--dump-plugins` surface).
//! * [`PluginHost`] — boot (dependency-gated activation with a
//!   fail-loud pending-service audit) and `shutdown()` (reverse-order
//!   disposal).
//!
//! Where new behavior goes (this table must keep holding):
//!
//! | Goal | Mechanism |
//! |---|---|
//! | Add tools | plugin row contributing `ToolContributor`s |
//! | Restrict tools per session | `restrict(scope, …)` overlay |
//! | Veto/rewrite tool policy | `tools/pre-execute` waterfall listener, or `guard()` |
//! | Observe tool outcomes | `tools/result` emit listener |
//! | Swap a service backend | `ServiceRegistry::replace` |
//! | Add cross-cutting behavior | bus listener on the matching catalog event |

pub mod catalog;
pub mod compose;
pub mod effects;
pub mod events;
pub mod host;
pub mod plugin;
pub mod scope;
pub mod services;
pub mod tools;

pub use catalog::{EVENT_CATALOG, EventDef, Mode};
pub use compose::{ComposedRow, Composition, Origin, PluginRegistry, Row, compose, dump};
pub use effects::{Effect, EffectStack};
pub use events::{BoxFuture, Bus, Listener, ListenerCb, Next, ScopeFilter};
pub use host::{BootReport, PluginHost};
pub use plugin::{Plugin, PluginContext, PluginInfo};
pub use scope::{ScopeChain, ScopeKey};
pub use services::{ServiceRegistry, ServiceError};
pub use tools::{
    PostDecision, PreDecision, ToolContributor, ToolExec, ToolFault, ToolFaultKind,
    ToolRestriction, ToolService, allow_value, ask_value, block_value, deny_value,
};
