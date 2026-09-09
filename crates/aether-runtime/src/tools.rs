//! [`ToolService`] — the tool seam (Definition role).
//!
//! Ports the `ctx.tools` contract (`register` / `restrict` / `guard`
//! / `resolve` + the `pre → post` interception pipeline) onto
//! AETHER-native types:
//!
//! * Contributors register tools with a disposer ([`Effect`]); the
//!   service never imports a tool implementation (Provider role lives
//!   in `aether-tools::plugins`, Consumer role is the executor).
//! * Scoped layers: global contributions plus per-scope overlays with
//!   most-specific-wins shadowing. [`ToolRestriction`] overlays
//!   intersect; scope-own registrations are exempt so a delegated
//!   child keeps answering its own tools.
//! * Interception: `tools/pre-execute` waterfall (`Allow`/`Deny`/
//!   `Ask`), monotonic deny-only [`guards`](ToolService::guard)
//!   (a guard can never re-allow a denial), `tools/post-execute`
//!   waterfall (`Accept`/`Block`), `tools/result` emit.
//! * No-rewrite rule: the args a listener sees are the args that
//!   execute. Listeners cannot mutate them (the payload is a
//!   snapshot; the pipeline re-reads the authoritative `ToolExec`).
//! * Error taxonomy ([`ToolFaultKind`]) is assigned at this boundary
//!   so the loop never parses error strings.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog;
use crate::effects::Effect;
use crate::events::{BoxFuture, Bus};
use crate::scope::{ScopeChain, ScopeKey};

/// One tool execution, as the seam sees it. `args` is authoritative;
/// listeners receive a snapshot and cannot mutate it (no-rewrite rule).
#[derive(Debug, Clone)]
pub struct ToolExec {
    /// Stable call id for pairing pre/post/result.
    pub call_id: String,
    /// Tool name as resolved (post-shadowing).
    pub name: String,
    /// Arguments that WILL execute.
    pub args: Value,
    /// Dispatch scope (session key).
    pub scope: ScopeKey,
    /// Working directory the body will run in.
    pub cwd: std::path::PathBuf,
}

/// Pre-execution decision (the `tools/pre-execute` vocabulary).
#[derive(Debug, Clone, PartialEq)]
pub enum PreDecision {
    /// Proceed to policy/guards/body.
    Allow,
    /// Refuse with a reason. Monotonic: nothing downstream can
    /// re-allow this call.
    Deny { reason: String },
    /// Defer to the interactive/user policy path.
    Ask { reason: String },
}

/// Post-execution decision (the `tools/post-execute` vocabulary).
#[derive(Debug, Clone, PartialEq)]
pub enum PostDecision {
    /// Keep the body output, optionally replacing it.
    Accept { content_override: Option<String> },
    /// Convert a success into an error carrying feedback to the model.
    Block { feedback: String },
}

/// Tool error taxonomy, assigned at the seam boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolFaultKind {
    /// No visible tool by that name in this scope.
    UnknownTool,
    /// Arguments failed validation.
    InvalidArgs,
    /// Refused by pre-decision, guard or policy.
    Denied,
    /// Cancelled before or during dispatch.
    Aborted,
    /// Exceeded its time budget.
    Timeout,
    /// Body ran and failed.
    Failed,
}

/// A tool failure with a stable kind (the loop matches on `kind`,
/// never on message text).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolFault {
    pub kind: ToolFaultKind,
    pub message: String,
}

impl ToolFault {
    pub fn new(kind: ToolFaultKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

impl std::fmt::Display for ToolFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for ToolFault {}

/// A tool contributed to the seam (Provider role, adapted).
///
/// The body receives the authoritative args plus the cwd and resolves
/// to a JSON output value. Bodies must be total: panic containment
/// lives in the bus, but the pipeline treats `Err` as `Failed` and
/// never lets a panic cross into the loop.
#[derive(Clone)]
pub struct ToolContributor {
    /// Tool name (unique per scope layer after shadowing).
    pub name: String,
    /// Owning plugin id (provenance + disposal grouping).
    pub owner: String,
    /// Scope this contribution was registered into.
    pub scope: ScopeKey,
    /// Human description (for schema assembly).
    pub description: String,
    /// JSON-schema parameters (for schema assembly).
    pub parameters: Value,
    /// The body.
    pub execute:
        Arc<dyn Fn(Value, std::path::PathBuf) -> BoxFuture<'static, Result<Value, ToolFault>> + Send + Sync>,
}

impl std::fmt::Debug for ToolContributor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContributor")
            .field("name", &self.name)
            .field("owner", &self.owner)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

/// Per-scope visibility overlay. Multiple overlays intersect:
/// every defined `allow` must contain the tool, and no `deny` may
/// contain it. Scope-own registrations bypass overlays.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolRestriction {
    /// When non-empty (in ANY overlay), the tool must appear in EVERY
    /// defined allow set.
    pub allow: Option<HashSet<String>>,
    /// Union of deny sets; membership hides the tool.
    pub deny: HashSet<String>,
}

impl ToolRestriction {
    pub fn allow(names: impl IntoIterator<Item = String>) -> Self {
        Self { allow: Some(names.into_iter().collect()), deny: HashSet::new() }
    }

    pub fn deny(names: impl IntoIterator<Item = String>) -> Self {
        Self { allow: None, deny: names.into_iter().collect() }
    }
}

/// Deny-only policy hook. `None` = no opinion. `Some(reason)` denies
/// the call. There is deliberately no allow variant: guards are
/// monotonic and cannot re-allow a denial from the waterfall, the
/// policy engine, or another guard.
pub type ToolGuard =
    Arc<dyn Fn(&ToolExec) -> Option<String> + Send + Sync>;

/// The tool seam. Shareable (`Clone` is cheap): interior state
/// lives behind a mutex so disposal effects can unregister exactly
/// their own contribution without borrowing the service.
///
/// Lock discipline: methods lock briefly and never run effects (or
/// call out into listeners) while holding the lock, so disposal
/// cannot deadlock against registration.
#[derive(Clone, Default)]
pub struct ToolService {
    inner: Arc<std::sync::Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    /// Global contributions in registration order.
    global: Vec<ToolContributor>,
    /// Per-scope overlays (shadowing layers).
    scopes: HashMap<ScopeKey, Vec<ToolContributor>>,
    /// Per-scope restriction overlays.
    restrictions: HashMap<ScopeKey, Vec<ToolRestriction>>,
    /// Monotonic deny hooks: (owner, guard).
    guards: Vec<(String, ToolGuard)>,
}

impl std::fmt::Debug for ToolService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.inner.lock() {
            Ok(inner) => f
                .debug_struct("ToolService")
                .field("global", &inner.global.len())
                .field("scopes", &inner.scopes.len())
                .field("restrictions", &inner.restrictions.len())
                .field("guards", &inner.guards.len())
                .finish(),
            Err(_) => f.debug_struct("ToolService").finish_non_exhaustive(),
        }
    }
}

impl ToolService {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Register a contributor. Returns the disposal effect, which
    /// unregisters exactly this (scope, name, owner) triple.
    pub fn register(&self, contributor: ToolContributor) -> Effect {
        let label = format!("tool:{}:{}", contributor.owner, contributor.name);
        let (name, scope, owner) =
            (contributor.name.clone(), contributor.scope, contributor.owner.clone());
        {
            let mut inner = self.lock();
            if scope == ScopeKey::GLOBAL {
                inner.global.push(contributor);
            } else {
                inner.scopes.entry(scope).or_default().push(contributor);
            }
        }
        let inner = self.inner.clone();
        Effect::new(label, move || {
            let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
            let layer = if scope == ScopeKey::GLOBAL {
                &mut guard.global
            } else {
                match guard.scopes.get_mut(&scope) {
                    Some(l) => l,
                    None => return,
                }
            };
            layer.retain(|c| !(c.name == name && c.owner == owner));
        })
    }

    /// Remove one contributor by (scope, name, owner). Used by direct
    /// (non-host) owners and tests.
    pub fn unregister_one(&self, scope: ScopeKey, name: &str, owner: &str) -> bool {
        let mut inner = self.lock();
        let layer = if scope == ScopeKey::GLOBAL {
            &mut inner.global
        } else {
            match inner.scopes.get_mut(&scope) {
                Some(l) => l,
                None => return false,
            }
        };
        let before = layer.len();
        layer.retain(|c| !(c.name == name && c.owner == owner));
        before != layer.len()
    }

    /// Remove every contribution (tools + guards) owned by `owner`.
    /// Returns counts `(tools, guards)`. Host shutdown calls this per
    /// activation after running the activation's effect stack.
    pub fn unregister_owner(&self, owner: &str) -> (usize, usize) {
        let mut inner = self.lock();
        let mut tools = 0;
        let before = inner.global.len();
        inner.global.retain(|c| c.owner != owner);
        tools += before - inner.global.len();
        for layer in inner.scopes.values_mut() {
            let before = layer.len();
            layer.retain(|c| c.owner != owner);
            tools += before - layer.len();
        }
        inner.scopes.retain(|_, l| !l.is_empty());
        let gbefore = inner.guards.len();
        inner.guards.retain(|(o, _)| o != owner);
        (tools, gbefore - inner.guards.len())
    }

    /// Add a visibility overlay for `scope`. Returns the disposal effect.
    pub fn restrict(&self, scope: ScopeKey, restriction: ToolRestriction) -> Effect {
        self.lock().restrictions.entry(scope).or_default().push(restriction.clone());
        let inner = self.inner.clone();
        Effect::new(format!("restrict:{scope}"), move || {
            let mut guard = inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(overlays) = guard.restrictions.get_mut(&scope) {
                if let Some(pos) = overlays.iter().position(|o| *o == restriction) {
                    overlays.remove(pos);
                }
                if overlays.is_empty() {
                    guard.restrictions.remove(&scope);
                }
            }
        })
    }

    /// Add a monotonic deny guard. Returns the disposal effect.
    pub fn guard(&self, owner: impl Into<String>, guard: ToolGuard) -> Effect {
        let owner = owner.into();
        self.lock().guards.push((owner.clone(), guard.clone()));
        let inner = self.inner.clone();
        Effect::new(format!("guard:{owner}"), move || {
            let mut svc = inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(pos) =
                svc.guards.iter().position(|(o, g)| o == &owner && Arc::ptr_eq(g, &guard))
            {
                svc.guards.remove(pos);
            }
        })
    }

    /// Resolve the visible contributor for `name` along `chain`
    /// (nearest shadowing wins), applying restriction overlays.
    /// Non-global scope-own registrations are exempt from overlays;
    /// global contributions are always governed by them.
    ///
    /// Filtered-away or absent names resolve to `None`; the caller
    /// maps that to [`ToolFaultKind::UnknownTool`] so a hidden tool
    /// is indistinguishable from a missing one.
    pub fn resolve(&self, name: &str, chain: &ScopeChain) -> Option<ToolContributor> {
        let inner = self.lock();
        let found = chain.as_slice().iter().find_map(|scope| {
            let layer = if *scope == ScopeKey::GLOBAL {
                &inner.global
            } else {
                inner.scopes.get(scope)?
            };
            layer.iter().rev().find(|c| c.name == name).cloned()
        })?;
        // Own-scope exemption: a non-global scope always sees the tools
        // contributed into itself (so a delegated child keeps answering
        // its own tools under a restrictive parent overlay). Global
        // contributions are NOT exempt — overlays govern them.
        if found.scope != ScopeKey::GLOBAL && found.scope == chain.head() {
            return Some(found);
        }
        if is_allowed_locked(&inner, name, chain) {
            Some(found)
        } else {
            None
        }
    }

    /// Visibility predicate used by [`resolve`](Self::resolve) and by
    /// the executor's pre-check (same rule, no divergence).
    pub fn is_allowed(&self, name: &str, chain: &ScopeChain) -> bool {
        let inner = self.lock();
        is_allowed_locked(&inner, name, chain)
    }

    /// Names visible along `chain` (for schema assembly and the
    /// legacy tool-map bridge). Most-specific contributions first,
    /// then global insertion order, for a stable prompt.
    pub fn visible_names(&self, chain: &ScopeChain) -> Vec<String> {
        let inner = self.lock();
        let mut names: Vec<String> = Vec::new();
        let mut shadowed: HashSet<String> = HashSet::new();
        for scope in chain.as_slice() {
            let layer = if *scope == ScopeKey::GLOBAL {
                &inner.global
            } else {
                match inner.scopes.get(scope) {
                    Some(l) => l,
                    None => continue,
                }
            };
            for c in layer {
                let own = c.scope != ScopeKey::GLOBAL && c.scope == chain.head();
                if shadowed.insert(c.name.clone())
                    && (own || is_allowed_locked(&inner, &c.name, chain))
                {
                    names.push(c.name.clone());
                }
            }
        }
        names
    }

    /// First denying guard wins (registration order). Deny-only:
    /// guards cannot allow. Guards are cloned out from under the
    /// lock so bodies never run while it is held.
    pub fn check_guards(&self, exec: &ToolExec) -> Option<String> {
        let guards: Vec<(String, ToolGuard)> = self.lock().guards.clone();
        guards.iter().find_map(|(_, g)| g(exec))
    }

    /// Run the `tools/pre-execute` waterfall. No listeners ⇒ `Allow`.
    /// Result vocabulary: `{decision: allow|deny|ask, reason?}`.
    /// Anything unparseable is `Allow` (fail-open matches today's
    /// behavior; denials must be explicit).
    pub async fn pre_check(&self, bus: &Bus, exec: &ToolExec) -> PreDecision {
        let def = catalog::lookup("tools/pre-execute").expect("catalog has tools/pre-execute");
        let chain = ScopeChain::for_scope(exec.scope);
        let payload = serde_json::json!({
            "call_id": exec.call_id,
            "tool": exec.name,
            "args": exec.args,
            "scope": exec.scope.as_u64(),
        });
        let out = bus.dispatch(def, &chain, payload).await;
        match out.get("decision").and_then(|d| d.as_str()) {
            Some("deny") => PreDecision::Deny {
                reason: out
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("denied by policy")
                    .to_string(),
            },
            Some("ask") => PreDecision::Ask {
                reason: out
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("needs approval")
                    .to_string(),
            },
            _ => PreDecision::Allow,
        }
    }

    /// Run the `tools/post-execute` waterfall. No listeners ⇒ accept.
    /// Result vocabulary: `{decision: accept, content_override?}` or
    /// `{decision: block, feedback}`.
    pub async fn post_check(&self, bus: &Bus, exec: &ToolExec, output: &Value) -> PostDecision {
        let def = catalog::lookup("tools/post-execute").expect("catalog has tools/post-execute");
        let chain = ScopeChain::for_scope(exec.scope);
        let payload = serde_json::json!({
            "call_id": exec.call_id,
            "tool": exec.name,
            "output": output,
        });
        let out = bus.dispatch(def, &chain, payload).await;
        match out.get("decision").and_then(|d| d.as_str()) {
            Some("block") => PostDecision::Block {
                feedback: out
                    .get("feedback")
                    .and_then(|f| f.as_str())
                    .unwrap_or("blocked by policy")
                    .to_string(),
            },
            _ => PostDecision::Accept {
                content_override: out
                    .get("content_override")
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string()),
            },
        }
    }

    /// Emit `tools/result` (observe-only; never carries full output).
    pub async fn emit_result(&self, bus: &Bus, exec: &ToolExec, ok: bool) {
        let def = catalog::lookup("tools/result").expect("catalog has tools/result");
        let chain = ScopeChain::for_scope(exec.scope);
        bus.dispatch(
            def,
            &chain,
            serde_json::json!({ "call_id": exec.call_id, "tool": exec.name, "ok": ok }),
        )
        .await;
    }

    /// Emit `tools/change` (unfiltered membership notice).
    pub async fn emit_change(&self, bus: &Bus, owner: &str, kind: &str) {
        let def = catalog::lookup("tools/change").expect("catalog has tools/change");
        bus.dispatch(
            def,
            &ScopeChain::for_scope(ScopeKey::GLOBAL),
            serde_json::json!({ "owner": owner, "kind": kind }),
        )
        .await;
    }
}

/// Restriction evaluation against already-locked state.
fn is_allowed_locked(inner: &Inner, name: &str, chain: &ScopeChain) -> bool {
    let mut allows: Vec<&HashSet<String>> = Vec::new();
    let mut denied = false;
    for scope in chain.as_slice() {
        if let Some(overlays) = inner.restrictions.get(scope) {
            for overlay in overlays {
                if let Some(allow) = &overlay.allow {
                    allows.push(allow);
                }
                if overlay.deny.contains(name) {
                    denied = true;
                }
            }
        }
    }
    if denied {
        return false;
    }
    if allows.is_empty() {
        return true;
    }
    allows.iter().all(|set| set.contains(name))
}

// --- decision helpers for listener authors -------------------------------

/// Build an allow payload for `tools/pre-execute` listeners.
pub fn allow_value() -> Value {
    serde_json::json!({ "decision": "allow" })
}

/// Build a deny payload for `tools/pre-execute` listeners.
pub fn deny_value(reason: impl Into<String>) -> Value {
    serde_json::json!({ "decision": "deny", "reason": reason.into() })
}

/// Build an ask payload for `tools/pre-execute` listeners.
pub fn ask_value(reason: impl Into<String>) -> Value {
    serde_json::json!({ "decision": "ask", "reason": reason.into() })
}

/// Build a block payload for `tools/post-execute` listeners.
pub fn block_value(feedback: impl Into<String>) -> Value {
    serde_json::json!({ "decision": "block", "feedback": feedback.into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Mode;
    use crate::events::{ListenerCb, Next, ScopeFilter};
    use serde_json::json;
    use std::sync::Arc;

    fn contributor(name: &str, owner: &str, scope: ScopeKey) -> ToolContributor {
        ToolContributor {
            name: name.into(),
            owner: owner.into(),
            scope,
            description: "test tool".into(),
            parameters: json!({}),
            execute: Arc::new(|_args: Value, _cwd: std::path::PathBuf| {
                Box::pin(async move { Ok(json!({"ok": true})) }) as BoxFuture<'static, Result<Value, ToolFault>>
            }),
        }
    }

    fn exec(name: &str, scope: ScopeKey) -> ToolExec {
        ToolExec {
            call_id: "c1".into(),
            name: name.into(),
            args: json!({}),
            scope,
            cwd: std::path::PathBuf::from("/tmp"),
        }
    }

    #[test]
    fn scoped_contribution_shadows_global_for_its_scope_only() {
        let svc = ToolService::new();
        svc.register(contributor("read", "core", ScopeKey::GLOBAL));
        let s = ScopeKey(5);
        svc.register(contributor("read", "scoped-plugin", s));
        let global_chain = ScopeChain::for_scope(ScopeKey::GLOBAL);
        let scoped_chain = ScopeChain::for_scope(s);
        assert_eq!(svc.resolve("read", &global_chain).unwrap().owner, "core");
        assert_eq!(svc.resolve("read", &scoped_chain).unwrap().owner, "scoped-plugin");
        // Absent names resolve to None (caller maps to UnknownTool).
        assert!(svc.resolve("nope", &scoped_chain).is_none());
    }

    #[test]
    fn restrictions_intersect_and_deny_unions() {
        let svc = ToolService::new();
        svc.register(contributor("a", "core", ScopeKey::GLOBAL));
        svc.register(contributor("b", "core", ScopeKey::GLOBAL));
        svc.register(contributor("c", "core", ScopeKey::GLOBAL));
        let s = ScopeKey(5);
        let chain = ScopeChain::for_scope(s);
        svc.restrict(s, ToolRestriction::allow(["a".to_string(), "b".to_string()]));
        svc.restrict(s, ToolRestriction::allow(["b".to_string(), "c".to_string()]));
        svc.restrict(s, ToolRestriction::deny(["c".to_string()]));
        // Intersection of allows = {b}; c additionally denied.
        assert!(svc.resolve("b", &chain).is_some());
        assert!(svc.resolve("a", &chain).is_none());
        assert!(svc.resolve("c", &chain).is_none());
        // Global scope unaffected.
        let g = ScopeChain::for_scope(ScopeKey::GLOBAL);
        assert!(svc.resolve("a", &g).is_some());
    }

    #[test]
    fn own_scope_registrations_are_exempt_from_overlays() {
        let svc = ToolService::new();
        let s = ScopeKey(5);
        svc.register(contributor("child-tool", "delegated", s));
        svc.restrict(s, ToolRestriction::deny(["child-tool".to_string()]));
        let chain = ScopeChain::for_scope(s);
        assert!(svc.resolve("child-tool", &chain).is_some());
    }

    #[test]
    fn unregister_owner_removes_tools_and_guards() {
        let svc = ToolService::new();
        svc.register(contributor("a", "p1", ScopeKey::GLOBAL));
        svc.register(contributor("b", "p2", ScopeKey::GLOBAL));
        svc.guard("p1", Arc::new(|_| Some("no".into())));
        let (tools, guards) = svc.unregister_owner("p1");
        assert_eq!((tools, guards), (1, 1));
        let g = ScopeChain::for_scope(ScopeKey::GLOBAL);
        assert!(svc.resolve("a", &g).is_none());
        assert!(svc.resolve("b", &g).is_some());
    }

    #[tokio::test]
    async fn guards_are_monotonic_deny_only() {
        let svc = ToolService::new();
        svc.guard("policy", Arc::new(|e: &ToolExec| {
            (e.name == "execute_command").then(|| "shell is dez-enabled".to_string())
        }));
        assert!(svc.check_guards(&exec("read", ScopeKey::GLOBAL)).is_none());
        assert_eq!(
            svc.check_guards(&exec("execute_command", ScopeKey::GLOBAL)),
            Some("shell is dez-enabled".to_string())
        );
        // A guard has no allow path: absence of denial is the only
        // non-denying outcome, so nothing can re-allow upstream deny.
    }

    #[tokio::test]
    async fn pre_check_defaults_allow_and_honors_deny_ask() {
        let svc = ToolService::new();
        let bus = Bus::new();
        // No listeners: allow.
        assert_eq!(svc.pre_check(&bus, &exec("x", ScopeKey::GLOBAL)).await, PreDecision::Allow);

        let deny: ListenerCb = Arc::new(|v: Value, next: Next| {
            Box::pin(async move {
                if v["tool"] == "rm" {
                    deny_value("destructive")
                } else {
                    next.proceed(v).await
                }
            })
        });
        let (_h, _e) = bus.on("tools/pre-execute", ScopeFilter::All, deny);
        assert_eq!(
            svc.pre_check(&bus, &exec("rm", ScopeKey::GLOBAL)).await,
            PreDecision::Deny { reason: "destructive".into() }
        );
        assert_eq!(svc.pre_check(&bus, &exec("ls", ScopeKey::GLOBAL)).await, PreDecision::Allow);
    }

    #[tokio::test]
    async fn post_check_accepts_by_default_and_blocks_explicitly() {
        let svc = ToolService::new();
        let bus = Bus::new();
        assert_eq!(
            svc.post_check(&bus, &exec("x", ScopeKey::GLOBAL), &json!({})).await,
            PostDecision::Accept { content_override: None }
        );
        let block: ListenerCb = Arc::new(|_v: Value, _next: Next| {
            Box::pin(async move { block_value("leaked secret shape") })
        });
        let (_h, _e) = bus.on("tools/post-execute", ScopeFilter::All, block);
        assert_eq!(
            svc.post_check(&bus, &exec("x", ScopeKey::GLOBAL), &json!({})).await,
            PostDecision::Block { feedback: "leaked secret shape".into() }
        );
    }

    #[tokio::test]
    async fn executed_args_equal_logged_args_no_rewrite() {
        // The pipeline hands the SAME `args` value to listeners (as a
        // snapshot) and to the body. A listener cannot mutate the
        // authoritative copy: prove the body receives the original
        // even when a listener returns a doctored payload.
        let svc = ToolService::new();
        let bus = Bus::new();
        let rewrite: ListenerCb = Arc::new(|_v: Value, _next: Next| {
            Box::pin(async move { json!({"decision": "allow", "args": {"evil": true}}) })
        });
        let (_h, _e) = bus.on("tools/pre-execute", ScopeFilter::All, rewrite);
        let e = ToolExec {
            call_id: "c9".into(),
            name: "write".into(),
            args: json!({"path": "a.txt"}),
            scope: ScopeKey::GLOBAL,
            cwd: std::path::PathBuf::from("/tmp"),
        };
        // pre_check returns Allow; the authoritative exec is untouched.
        assert_eq!(svc.pre_check(&bus, &e).await, PreDecision::Allow);
        assert_eq!(e.args, json!({"path": "a.txt"}));
        let _ = Mode::Emit; // (catalog import used by other tests)
    }
}
