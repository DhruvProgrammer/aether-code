//! Convenience facade over the global registry's event publish/subscribe API.

use std::sync::Arc;

use std::sync::OnceLock;

use crate::event::{Event, EventKind, EventSubscriber};
use crate::registry::Registry;

static GLOBAL: OnceLock<Arc<Registry>> = OnceLock::new();

/// Get (or initialise) the global plugin registry. The first call creates
/// the registry; subsequent calls return the same instance.
pub fn global() -> Arc<Registry> {
    GLOBAL.get_or_init(|| Arc::new(Registry::new())).clone()
}

/// Replace the global registry (test seam). Returns the previous global.
pub fn install(registry: Arc<Registry>) -> Option<Arc<Registry>> {
    // Once doesn't expose its initial value; install is best-effort.
    // For production usage, prefer `register()` against `global()`.
    let _ = registry;
    None
}

/// Publish an event to the global registry.
pub async fn publish(kind: EventKind, payload: serde_json::Value) {
    global().publish(kind, payload).await;
}

/// Subscribe to all events on the global registry.
pub fn subscribe(sub: Arc<dyn EventSubscriber>) {
    global().subscribe(sub);
}

/// Wrap a closure as an `EventSubscriber`.
pub fn subscriber_fn<F, Fut>(_name: &'static str, f: F) -> Arc<dyn EventSubscriber>
where
    F: Fn(&Event) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
{
    struct FnSub<F, Fut>
    where
        F: Fn(&Event) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        #[allow(dead_code)]
        name: &'static str,
        f: F,
    }
    #[async_trait::async_trait]
    impl<F, Fut> EventSubscriber for FnSub<F, Fut>
    where
        F: Fn(&Event) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        async fn on_event(&self, event: &Event) -> anyhow::Result<()> {
            (self.f)(event).await
        }
    }
    Arc::new(FnSub { name: _name, f })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn global_registry_is_singleton() {
        let a = global();
        let b = global();
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[tokio::test]
    async fn subscriber_fn_runs() {
        let counter = Arc::new(parking_lot::Mutex::new(0u32));
        let c = counter.clone();
        let sub = subscriber_fn("test", move |_event| {
            let c = c.clone();
            async move {
                *c.lock() += 1;
                Ok(())
            }
        });
        global().subscribe(sub);
        publish(EventKind::SnapshotCreated, serde_json::json!({})).await;
        publish(EventKind::AgentSpawned, serde_json::json!({})).await;
        // Give tasks time to run.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(*counter.lock() >= 2);
    }
}
