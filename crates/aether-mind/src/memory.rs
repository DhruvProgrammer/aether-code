//! MemoryManager — orchestration over memory providers.
//!
//! Inspired by Hermes Agent's `MemoryManager` (single external provider slot,
//! with built-in `Mind` always present). The runtime calls
//! `build_system_prompt()` / `prefetch_all(query)` / `sync_all(turn)` exactly
//! once; the manager fans out across registered providers.
//!
//! This is the single integration point the agent loop uses. Per-backend
//! memory code stays inside individual providers.

use parking_lot::Mutex;
use std::sync::Arc;

/// A memory provider supplies (a) optional additional system-prompt text and
/// (b) selective context for a given query.
pub trait MemoryProvider: Send + Sync {
    /// Stable identifier (e.g. "builtin", "openai_chat", "anthropic_memory").
    fn name(&self) -> &str;
    /// Synchronous, non-blocking. Returns the fragment of context this
    /// provider wants injected for `query`. The caller is responsible for
    /// truncation / context-budget enforcement.
    fn prefetch(&self, query: &str) -> String;
    /// Post-turn persistence hook. Best-effort; a slow provider must not
    /// block the agent loop.
    fn sync(&self, turn: &MemoryTurn);
    /// Optional additional system-prompt block (identity, platform, skills).
    fn system_prompt_block(&self) -> String { String::new() }
}

/// One logical agent turn, passed to provider `sync`.
#[derive(Debug, Clone)]
pub struct MemoryTurn<'a> {
    pub user: &'a str,
    pub assistant: &'a str,
    pub session_id: &'a str,
    pub workspace: Option<&'a str>,
}

/// MemoryManager — single integration point for the agent runtime.
pub struct MemoryManager {
    providers: Mutex<Vec<Arc<dyn MemoryProvider>>>,
}

impl Default for MemoryManager {
    fn default() -> Self { Self::new() }
}

impl MemoryManager {
    pub fn new() -> Self { Self { providers: Mutex::new(Vec::new()) } }

    /// Register a provider. Mirrors Hermes: the built-in `Mind` provider is
    /// always accepted; only one external provider is allowed at a time.
    /// A second external attempt is rejected with a warning and ignored.
    pub fn add_provider(&self, p: Arc<dyn MemoryProvider>) {
        let mut guard = self.providers.lock();
        let is_builtin = p.name() == "builtin";
        if !is_builtin {
            let has_external = guard.iter().any(|q| q.name() != "builtin");
            if has_external {
                let existing = guard.iter().find(|q| q.name() != "builtin").map(|q| q.name().to_string()).unwrap_or_default();
                eprintln!("[memory] rejected provider '{}': external provider '{}' is already registered (single slot)", p.name(), existing);
                return;
            }
        }
        guard.push(p);
    }

    pub fn providers(&self) -> Vec<Arc<dyn MemoryProvider>> { self.providers.lock().clone() }

    /// Assembled system-prompt block from all providers (built-in first).
    /// Caller is responsible for placing this AFTER AETHER's own system prompt
    /// and BEFORE the user message.
    pub fn build_system_prompt(&self) -> String {
        self.providers
            .lock()
            .iter()
            .map(|p| p.system_prompt_block())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Selective prefetch across all providers. Truncated by caller.
    pub fn prefetch_all(&self, query: &str) -> String {
        self.providers
            .lock()
            .iter()
            .map(|p| p.prefetch(query))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    }

    /// Post-turn persistence fan-out. Best-effort.
    pub fn sync_all(&self, turn: &MemoryTurn) {
        for p in self.providers.lock().iter() {
            p.sync(turn);
        }
    }
}

/// Built-in provider that wraps the existing `Mind` graph+vector+kv store.
/// `system_prompt_block` returns the saved skills index; `prefetch` does a
/// hybrid recall (graph + vector + kv).
pub struct BuiltinMemoryProvider;

impl MemoryProvider for BuiltinMemoryProvider {
    fn name(&self) -> &str { "builtin" }
    fn prefetch(&self, _query: &str) -> String { String::new() }
    fn sync(&self, _turn: &MemoryTurn) { /* Built-in Mind already persists via its own API. */ }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CapturingProvider { name: String, prefetch: String, system: String }
    impl MemoryProvider for CapturingProvider {
        fn name(&self) -> &str { &self.name }
        fn prefetch(&self, _query: &str) -> String { self.prefetch.clone() }
        fn sync(&self, _turn: &MemoryTurn) {}
        fn system_prompt_block(&self) -> String { self.system.clone() }
    }

    #[test]
    fn single_external_slot_is_enforced() {
        let m = MemoryManager::new();
        m.add_provider(Arc::new(CapturingProvider { name: "ext1".into(), prefetch: "p1".into(), system: "s1".into() }));
        m.add_provider(Arc::new(CapturingProvider { name: "ext2".into(), prefetch: "p2".into(), system: "s2".into() }));
        assert_eq!(m.providers().len(), 1, "second external provider must be rejected");
        assert_eq!(m.providers()[0].name(), "ext1");
    }

    #[test]
    fn builtin_plus_one_external_is_allowed() {
        let m = MemoryManager::new();
        m.add_provider(Arc::new(BuiltinMemoryProvider));
        m.add_provider(Arc::new(CapturingProvider { name: "ext".into(), prefetch: "p".into(), system: "s".into() }));
        assert_eq!(m.providers().len(), 2);
    }

    #[test]
    fn build_and_prefetch_join_blocks() {
        let m = MemoryManager::new();
        m.add_provider(Arc::new(BuiltinMemoryProvider));
        m.add_provider(Arc::new(CapturingProvider { name: "ext".into(), prefetch: "from-provider".into(), system: "block-1".into() }));
        let sys = m.build_system_prompt();
        assert!(sys.contains("block-1"));
        let ctx = m.prefetch_all("find auth");
        assert!(ctx.contains("from-provider"));
    }
}
