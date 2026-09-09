//! [`ScopeKey`] — opaque scope identity for scoped registration and
//! scope-filtered dispatch.
//!
//! Port of the `dsh-scope` primitive (`ScopeKey = object`, identity
//! compared): the global scope plus one minted key per session (later:
//! per agent). Children list their ancestors so an ancestor listener
//! observes descendant dispatches, while contributions inherit
//! downward (child sees ancestors' layers).

/// Opaque scope identity. `0` is reserved for the global scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopeKey(pub u64);

impl ScopeKey {
    /// The process-global scope. Contributions registered here are
    /// visible to every scope unless shadowed or restricted.
    pub const GLOBAL: ScopeKey = ScopeKey(0);

    /// Render for event payloads and diagnostics.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for ScopeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if *self == ScopeKey::GLOBAL {
            write!(f, "global")
        } else {
            write!(f, "scope:{}", self.0)
        }
    }
}

/// An ordered dispatch/resolution chain, most-specific first.
///
/// Today every session scope chains to exactly `[self, GLOBAL]`;
/// the type already carries the general shape (nested delegation,
/// preset generations) so P2/P3 do not need a signature change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeChain {
    chain: Vec<ScopeKey>,
}

impl ScopeChain {
    /// Chain for a session scope: itself, then global.
    pub fn for_scope(key: ScopeKey) -> Self {
        if key == ScopeKey::GLOBAL {
            Self { chain: vec![ScopeKey::GLOBAL] }
        } else {
            Self { chain: vec![key, ScopeKey::GLOBAL] }
        }
    }

    /// Chain from an explicit ancestor list (most-specific first).
    /// An empty list resolves to the global scope.
    pub fn from_ancestors(mut ancestors: Vec<ScopeKey>) -> Self {
        if ancestors.is_empty() {
            ancestors.push(ScopeKey::GLOBAL);
        } else if !ancestors.contains(&ScopeKey::GLOBAL) {
            ancestors.push(ScopeKey::GLOBAL);
        }
        Self { chain: ancestors }
    }

    /// Most-specific scope first.
    pub fn as_slice(&self) -> &[ScopeKey] {
        &self.chain
    }

    /// The scope dispatch originates from.
    pub fn head(&self) -> ScopeKey {
        self.chain[0]
    }

    /// True when `key` is the dispatch head or one of its ancestors
    /// (ancestor listeners observe descendant dispatches).
    pub fn admits(&self, key: ScopeKey) -> bool {
        self.chain.contains(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_chain_is_just_global() {
        let c = ScopeChain::for_scope(ScopeKey::GLOBAL);
        assert_eq!(c.as_slice(), &[ScopeKey::GLOBAL]);
    }

    #[test]
    fn session_chain_admits_self_and_global_only() {
        let s = ScopeKey(7);
        let c = ScopeChain::for_scope(s);
        assert_eq!(c.head(), s);
        assert!(c.admits(s));
        assert!(c.admits(ScopeKey::GLOBAL));
        assert!(!c.admits(ScopeKey(8)));
    }

    #[test]
    fn ancestors_always_end_at_global() {
        let c = ScopeChain::from_ancestors(vec![ScopeKey(3), ScopeKey(2)]);
        assert_eq!(c.as_slice(), &[ScopeKey(3), ScopeKey(2), ScopeKey::GLOBAL]);
        let empty = ScopeChain::from_ancestors(vec![]);
        assert_eq!(empty.as_slice(), &[ScopeKey::GLOBAL]);
    }
}
