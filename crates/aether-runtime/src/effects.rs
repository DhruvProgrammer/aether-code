//! Reversible effects: every registration returns a disposer, and
//! shutdown unwinds them in reverse order.
//!
//! Port of the Cordis rule "registrations are effects": a registry's
//! `register()` returns the exact disposer, and fiber/group disposal
//! unwinds in reverse so reload and teardown are predictable.

/// A single reversible registration. The undo closure runs at most
/// once; dropping an `Effect` without running it is a no-op (never a
/// leak — the host drains every stack on shutdown, and tests assert
/// empty stacks).
pub struct Effect {
    label: String,
    undo: Option<Box<dyn FnOnce() + Send>>,
}

impl Effect {
    /// Create an effect with a human-readable label (used in
    /// leak/debug reports).
    pub fn new(label: impl Into<String>, undo: impl FnOnce() + Send + 'static) -> Self {
        Self { label: label.into(), undo: Some(Box::new(undo)) }
    }

    /// A no-op effect, for registrations that need no teardown.
    pub fn noop(label: impl Into<String>) -> Self {
        Self { label: label.into(), undo: None }
    }

    /// Run the undo closure once. Idempotent: second call is a no-op
    /// returning `false`.
    pub fn run(&mut self) -> bool {
        match self.undo.take() {
            Some(undo) => {
                undo();
                true
            }
            None => false,
        }
    }

    /// True when the undo closure has already run (or never existed).
    pub fn is_spent(&self) -> bool {
        self.undo.is_none()
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

impl std::fmt::Debug for Effect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Effect")
            .field("label", &self.label)
            .field("spent", &self.is_spent())
            .finish()
    }
}

/// An ordered stack of effects, unwound in reverse (LIFO).
#[derive(Debug, Default)]
pub struct EffectStack {
    effects: Vec<Effect>,
}

impl EffectStack {
    pub fn new() -> Self {
        Self { effects: Vec::new() }
    }

    /// Push a registration's disposer. Returns the new depth.
    pub fn push(&mut self, effect: Effect) -> usize {
        self.effects.push(effect);
        self.effects.len()
    }

    /// Unwind all effects in reverse order. Returns the labels in
    /// unwind order (useful for disposal-order assertions).
    pub fn unwind(&mut self) -> Vec<String> {
        let mut order = Vec::with_capacity(self.effects.len());
        while let Some(mut effect) = self.effects.pop() {
            order.push(effect.label().to_string());
            effect.run();
        }
        order
    }

    /// Labels of still-live effects (non-spent), oldest first.
    /// A healthy shutdown leaves this empty.
    pub fn live_labels(&self) -> Vec<&str> {
        self.effects.iter().filter(|e| !e.is_spent()).map(|e| e.label()).collect()
    }

    pub fn len(&self) -> usize {
        self.effects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn unwind_runs_in_reverse_and_is_idempotent() {
        let log: Arc<Mutex<Vec<&str>>> = Arc::new(Mutex::new(Vec::new()));
        let mut stack = EffectStack::new();
        for label in ["first", "second", "third"] {
            let log = log.clone();
            stack.push(Effect::new(label, move || log.lock().unwrap().push(label)));
        }
        let order = stack.unwind();
        assert_eq!(order, vec!["third", "second", "first"]);
        assert_eq!(*log.lock().unwrap(), vec!["third", "second", "first"]);
        // Second unwind is a no-op.
        assert!(stack.unwind().is_empty());
        assert!(stack.live_labels().is_empty());
    }

    #[test]
    fn single_effect_runs_once() {
        let mut e = Effect::new("x", || {});
        assert!(!e.is_spent());
        assert!(e.run());
        assert!(e.is_spent());
        assert!(!e.run());
    }
}
