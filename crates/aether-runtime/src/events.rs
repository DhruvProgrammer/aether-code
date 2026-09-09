//! Mode-tagged event bus with scope-filtered dispatch.
//!
//! Port of the Cordis dispatch contract (`emit` / `waterfall` /
//! `parallel` / `serial` + scope carriers):
//!
//! * `Emit` — observe-only. Listeners run to completion, results are
//!   discarded, and a panicking listener cannot break dispatch.
//! * `Waterfall` — around-middleware. Listeners run in registration
//!   order and each receives a [`Next`] continuation. Calling
//!   `next.proceed(value)` delegates (possibly wrapped on the way
//!   back); returning without delegating short-circuits the chain.
//!   Policy owns the decision — this is how approval, timeout and
//!   guard plugins attach without loop edits.
//! * `Serial` — awaited in order, results collected (terminal
//!   checkpoints such as turn-stopping).
//! * `Parallel` — fanned out on the Tokio runtime and awaited as a
//!   set (flush-style barriers). Join failures become `Null`.
//!
//! Scope filtering: every dispatch carries a [`ScopeChain`]. A
//! listener tagged [`ScopeFilter::All`] (untagged in Cordis terms)
//! sees everything; [`ScopeFilter::Key`] admits dispatches whose
//! chain contains the key (self + descendants); events declared
//! `unfiltered` (registry-subject notifications such as
//! `tools/change`) bypass filters entirely.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::catalog::EventDef;
use crate::scope::{ScopeChain, ScopeKey};

/// Boxed future used for listener callbacks.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Listener callback: current payload plus the [`Next`] continuation.
/// Non-waterfall modes still receive a `Next`, but it is inert
/// (`proceed` returns the value unchanged).
pub type ListenerCb =
    Arc<dyn Fn(Value, Next) -> BoxFuture<'static, Value> + Send + Sync>;

/// Which dispatches a listener wants to observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeFilter {
    /// See every dispatch of this event.
    All,
    /// Only dispatches originating from the global scope.
    GlobalOnly,
    /// Dispatches whose chain contains this key (self + descendants).
    Key(ScopeKey),
}

impl ScopeFilter {
    fn admits(&self, chain: &ScopeChain) -> bool {
        match *self {
            ScopeFilter::All => true,
            ScopeFilter::GlobalOnly => chain.as_slice() == [ScopeKey::GLOBAL],
            ScopeFilter::Key(k) => chain.admits(k),
        }
    }
}

/// The waterfall continuation. `proceed(value)` runs the rest of the
/// chain and resolves to the downstream result, so a listener can
/// wrap it (`let out = next.proceed(v).await; …`) or ignore it to
/// short-circuit.
#[derive(Clone)]
pub struct Next {
    chain: Arc<Chain>,
    idx: usize,
}

impl Next {
    /// Delegate to the rest of the chain.
    pub async fn proceed(self, value: Value) -> Value {
        run_waterfall(&self.chain, self.idx, value).await
    }

    /// An inert continuation (returns its input unchanged).
    pub fn inert() -> Self {
        Self { chain: Arc::new(Chain { listeners: Vec::new() }), idx: 0 }
    }
}

impl std::fmt::Debug for Next {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Next").field("idx", &self.idx).finish()
    }
}

struct Chain {
    listeners: Vec<ListenerCb>,
}

fn run_waterfall(chain: &Arc<Chain>, idx: usize, value: Value) -> BoxFuture<'static, Value> {
    let chain = chain.clone();
    Box::pin(async move {
        match chain.listeners.get(idx) {
            None => value,
            Some(cb) => {
                let next = Next { chain: chain.clone(), idx: idx + 1 };
                // Contained: a panicking listener continues the chain
                // with the value unchanged instead of aborting dispatch.
                match catch_await(cb(value.clone(), next)).await {
                    Ok(v) => v,
                    Err(_) => run_waterfall(&chain, idx + 1, value).await,
                }
            }
        }
    })
}

struct Subscription {
    id: u64,
    event: &'static str,
    filter: ScopeFilter,
    cb: ListenerCb,
}

struct BusInner {
    listeners: Mutex<Vec<Subscription>>,
    next_id: AtomicU64,
}

/// Mode-tagged bus. Cheap to clone (`Arc` inside); the host owns one.
#[derive(Clone, Default)]
pub struct Bus {
    inner: Arc<BusInner>,
}

impl std::fmt::Debug for Bus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bus").finish_non_exhaustive()
    }
}

impl Default for BusInner {
    fn default() -> Self {
        Self { listeners: Mutex::new(Vec::new()), next_id: AtomicU64::new(1) }
    }
}

/// A listener registration handle. Kept for symmetry with [`Effect`];
/// dropping it does NOT unsubscribe — disposal runs the matching
/// effect returned by [`Bus::on`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listener {
    /// Listener id (also used as the disposal effect label suffix).
    pub id: u64,
    /// Event name it observes.
    pub event: &'static str,
}

impl Bus {
    pub fn new() -> Self {
        Self { inner: Arc::new(BusInner::default()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Subscription>> {
        self.inner.listeners.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Subscribe. Returns the handle plus the disposal [`Effect`]
    /// (running it unsubscribes exactly this listener).
    pub fn on(
        &self,
        event: &'static str,
        filter: ScopeFilter,
        cb: ListenerCb,
    ) -> (Listener, crate::effects::Effect) {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        self.lock().push(Subscription { id, event, filter, cb });
        let inner = self.inner.clone();
        let effect = crate::effects::Effect::new(format!("listener:{event}:{id}"), move || {
            let mut listeners =
                inner.listeners.lock().unwrap_or_else(|e| e.into_inner());
            listeners.retain(|l| l.id != id);
        });
        (Listener { id, event }, effect)
    }

    fn admitted(&self, def: &EventDef, chain: &ScopeChain) -> Vec<ListenerCb> {
        self.lock()
            .iter()
            .filter(|l| {
                l.event == def.name && (def.unfiltered || l.filter.admits(chain))
            })
            .map(|l| l.cb.clone())
            .collect()
    }

    /// Dispatch per `def.mode`. See the module docs for semantics.
    /// Every mode contains panicking listeners (failed polls become
    /// the value unchanged for `Waterfall`, `Null` otherwise).
    pub async fn dispatch(&self, def: &EventDef, chain: &ScopeChain, payload: Value) -> Value {
        let listeners = self.admitted(def, chain);
        match def.mode {
            crate::catalog::Mode::Emit => {
                for cb in listeners {
                    let _ = catch_await(cb(payload.clone(), Next::inert())).await;
                }
                Value::Null
            }
            crate::catalog::Mode::Serial => {
                let mut out = Vec::with_capacity(listeners.len());
                for cb in listeners {
                    out.push(
                        catch_await(cb(payload.clone(), Next::inert()))
                            .await
                            .unwrap_or(Value::Null),
                    );
                }
                Value::Array(out)
            }
            crate::catalog::Mode::Parallel => {
                let mut handles = Vec::with_capacity(listeners.len());
                for cb in listeners {
                    let payload = payload.clone();
                    handles.push(tokio::spawn(async move {
                        catch_await(cb(payload, Next::inert())).await.unwrap_or(Value::Null)
                    }));
                }
                let mut out = Vec::with_capacity(handles.len());
                for h in handles {
                    out.push(h.await.unwrap_or(Value::Null));
                }
                Value::Array(out)
            }
            crate::catalog::Mode::Waterfall => {
                let chain = Arc::new(Chain { listeners });
                run_waterfall(&chain, 0, payload).await
            }
        }
    }

    /// Number of live listeners (tests and boot reports).
    pub fn listener_count(&self) -> usize {
        self.lock().len()
    }
}

// --- async panic containment -------------------------------------------
// No executor-agnostic async `catch_unwind` exists on stable without an
// extra crate. Driving each listener inside a spawned task gives the
// same guarantee with a dependency we already have: a panicking
// listener fails its `JoinHandle`, dispatch observes `Err` and carries
// on. Deterministic modes (Emit/Serial/Waterfall) still run in order.
async fn catch_await<F>(fut: F) -> Result<F::Output, ()>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let handle = tokio::spawn(async move { fut.await });
    handle.await.map_err(|_| ())
}

#[cfg(test)]
mod tests {
    // (Real behavior tests live here; they use `catch_await` so a
    // panicking listener can never break dispatch.)
    use super::*;
    use crate::catalog::Mode;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    fn def(name: &'static str, mode: Mode) -> EventDef {
        EventDef { name, mode, unfiltered: false, doc: "test" }
    }

    fn echo(tag: &'static str) -> ListenerCb {
        Arc::new(move |v: Value, next: Next| {
            Box::pin(async move {
                let mut v = next.proceed(v).await;
                if let Value::Object(ref mut m) = v {
                    m.insert("seen".into(), Value::String(tag.into()));
                }
                v
            })
        })
    }

    #[tokio::test]
    async fn waterfall_delegates_in_order_and_wraps_on_return() {
        let bus = Bus::new();
        let d = def("tools/pre-execute", Mode::Waterfall);
        let chain = ScopeChain::for_scope(ScopeKey::GLOBAL);
        let _a = bus.on(d.name, ScopeFilter::All, echo("first"));
        let _b = bus.on(d.name, ScopeFilter::All, echo("second"));
        let out = bus.dispatch(&d, &chain, json!({"n": 1})).await;
        // Innermost ("second") returns first, so its marker wins;
        // both ran (no short-circuit).
        assert_eq!(out["seen"], json!("first"));
        assert_eq!(out["n"], json!(1));
    }

    #[tokio::test]
    async fn waterfall_short_circuit_skips_downstream() {
        let bus = Bus::new();
        let d = def("tools/pre-execute", Mode::Waterfall);
        let chain = ScopeChain::for_scope(ScopeKey::GLOBAL);
        let ran: Arc<StdMutex<Vec<&'static str>>> = Arc::new(StdMutex::new(vec![]));
        let ran2 = ran.clone();
        let veto: ListenerCb = Arc::new(move |_v: Value, _next: Next| {
            let ran2 = ran2.clone();
            Box::pin(async move {
                ran2.lock().unwrap().push("veto");
                json!({"decision": "deny"})
            })
        });
        let _v = bus.on(d.name, ScopeFilter::All, veto);
        let ran_assert = ran.clone();
        let downstream: ListenerCb = Arc::new(move |v: Value, next: Next| {
            let ran = ran.clone();
            Box::pin(async move {
                ran.lock().unwrap().push("downstream");
                next.proceed(v).await
            })
        });
        let _w = bus.on(d.name, ScopeFilter::All, downstream);
        let out = bus.dispatch(&d, &chain, json!({})).await;
        assert_eq!(out, json!({"decision": "deny"}));
        assert_eq!(*ran_assert.lock().unwrap(), vec!["veto"]);
    }

    fn tag_listener(tag: &'static str, hits: Arc<StdMutex<Vec<&'static str>>>) -> ListenerCb {
        Arc::new(move |v: Value, _n: Next| {
            let hits = hits.clone();
            let fut = async move {
                hits.lock().unwrap().push(tag);
                v
            };
            Box::pin(fut) as BoxFuture<'static, Value>
        })
    }

    #[tokio::test]
    async fn scope_filter_admits_self_descendants_and_global() {
        let bus = Bus::new();
        let d = def("tools/pre-execute", Mode::Serial);
        let hits: Arc<StdMutex<Vec<&'static str>>> = Arc::new(StdMutex::new(vec![]));
        let _ = bus.on(d.name, ScopeFilter::All, tag_listener("all", hits.clone()));
        let _ = bus.on(d.name, ScopeFilter::GlobalOnly, tag_listener("g", hits.clone()));
        let _ = bus.on(d.name, ScopeFilter::Key(ScopeKey(9)), tag_listener("s9", hits.clone()));
        let _ = bus.on(d.name, ScopeFilter::Key(ScopeKey(10)), tag_listener("s10", hits.clone()));
        let chain = ScopeChain::for_scope(ScopeKey(9));
        bus.dispatch(&d, &chain, json!(null)).await;
        let mut got = hits.lock().unwrap().clone();
        got.sort();
        assert_eq!(got, vec!["all", "s9"]);
    }

    #[tokio::test]
    async fn unfiltered_events_bypass_scope_filters() {
        let bus = Bus::new();
        let d = EventDef { name: "tools/change", mode: Mode::Emit, unfiltered: true, doc: "t" };
        let hits: Arc<StdMutex<u32>> = Arc::new(StdMutex::new(0));
        let h = hits.clone();
        let _ = bus.on(
            d.name,
            ScopeFilter::Key(ScopeKey(42)),
            Arc::new(move |v: Value, _n: Next| {
                let h = h.clone();
                Box::pin(async move {
                    *h.lock().unwrap() += 1;
                    v
                })
            }),
        );
        bus.dispatch(&d, &ScopeChain::for_scope(ScopeKey(7)), json!(null)).await;
        assert_eq!(*hits.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn emit_contains_panicking_listener() {
        let bus = Bus::new();
        let d = def("tools/result", Mode::Emit);
        let hits: Arc<StdMutex<u32>> = Arc::new(StdMutex::new(0));
        let _ = bus.on(
            d.name,
            ScopeFilter::All,
            Arc::new(move |_v: Value, _n: Next| {
                let fut = async move {
                    panic!("boom");
                    #[allow(unreachable_code)]
                    Value::Null
                };
                Box::pin(fut) as BoxFuture<'static, Value>
            }),
        );
        let h = hits.clone();
        let _ = bus.on(
            d.name,
            ScopeFilter::All,
            Arc::new(move |v: Value, _n: Next| {
                let h = h.clone();
                Box::pin(async move {
                    *h.lock().unwrap() += 1;
                    v
                })
            }),
        );
        bus.dispatch(&d, &ScopeChain::for_scope(ScopeKey::GLOBAL), json!(null)).await;
        assert_eq!(*hits.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn parallel_runs_all_and_collects() {
        let bus = Bus::new();
        let d = def("host/flush", Mode::Parallel);
        for i in 0..4 {
            let _ = bus.on(
                d.name,
                ScopeFilter::All,
                Arc::new(move |_v: Value, _n: Next| {
                    Box::pin(async move { json!(i) }) as BoxFuture<'static, Value>
                }),
            );
        }
        let out = bus.dispatch(&d, &ScopeChain::for_scope(ScopeKey::GLOBAL), json!(null)).await;
        let mut nums: Vec<i64> = out.as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
        nums.sort();
        assert_eq!(nums, vec![0, 1, 2, 3]);
    }

    #[tokio::test]
    async fn disposal_unsubscribes_exactly() {
        let bus = Bus::new();
        let d = def("tools/result", Mode::Serial);
        let hits: Arc<StdMutex<u32>> = Arc::new(StdMutex::new(0));
        let h = hits.clone();
        let (_, mut effect) = bus.on(
            d.name,
            ScopeFilter::All,
            Arc::new(move |v: Value, _n: Next| {
                let h = h.clone();
                Box::pin(async move {
                    *h.lock().unwrap() += 1;
                    v
                })
            }),
        );
        assert_eq!(bus.listener_count(), 1);
        assert!(effect.run());
        assert!(!effect.run());
        assert_eq!(bus.listener_count(), 0);
        bus.dispatch(&d, &ScopeChain::for_scope(ScopeKey::GLOBAL), json!(null)).await;
        assert_eq!(*hits.lock().unwrap(), 0);
    }
}
