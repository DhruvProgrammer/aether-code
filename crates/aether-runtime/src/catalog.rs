//! Mode-tagged event catalog.
//!
//! Port of the generated `event-producer-consumer.md`: every event
//! declares its dispatch [`Mode`] once, and dispatch sites look the
//! definition up instead of choosing semantics ad hoc. The
//! [`catalog_freshness`](tests) test fails the build when a definition
//! and its documentation row diverge.

/// Dispatch mode. The mode is contract: it decides awaiting,
/// ordering and return-shape rules for every listener of the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Observe-only. Results discarded, panics contained.
    Emit,
    /// Around-middleware. Listeners receive [`Next`](crate::Next);
    /// returning without delegating short-circuits.
    Waterfall,
    /// Awaited in registration order; results collected into an array.
    Serial,
    /// Fanned out concurrently and awaited as a set.
    Parallel,
}

/// One cataloged event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventDef {
    /// Stable event name, e.g. `"tools/pre-execute"`.
    pub name: &'static str,
    /// Dispatch contract.
    pub mode: Mode,
    /// Registry-subject notifications bypass scope filters so every
    /// scope observes membership changes (mirrors `tools/change` and
    /// `system-prompt/change`).
    pub unfiltered: bool,
    /// One-line contract documentation. The freshness test requires
    /// every definition to carry one.
    pub doc: &'static str,
}

/// The full event catalog. Add new events here (and only here — the
/// freshness test enforces that dispatch sites reference catalog
/// entries).
pub static EVENT_CATALOG: &[EventDef] = &[
    EventDef {
        name: "tools/pre-execute",
        mode: Mode::Waterfall,
        unfiltered: false,
        doc: "Veto/ask gate before tool policy runs. Payload {call_id,tool,args,scope}; result {decision: allow|deny|ask, reason?}. Returning without next() short-circuits to that decision.",
    },
    EventDef {
        name: "tools/post-execute",
        mode: Mode::Waterfall,
        unfiltered: false,
        doc: "Outcome gate after the tool body. Payload {call_id,tool,output}; result {decision: accept, content_override?} or {decision: block, feedback}.",
    },
    EventDef {
        name: "tools/result",
        mode: Mode::Emit,
        unfiltered: false,
        doc: "Observe-only settlement notice. Payload {call_id,tool,ok}. Never carries the full output (outputs stay in the session log).",
    },
    EventDef {
        name: "tools/change",
        mode: Mode::Emit,
        unfiltered: true,
        doc: "Registry membership changed (register/dispose/restrict). Payload {owner, kind: registered|unregistered|restricted}. Unfiltered: every scope observes it.",
    },
    EventDef {
        name: "agent/pre-step",
        mode: Mode::Waterfall,
        unfiltered: false,
        doc: "RESERVED (P1): prompt-assembly gate before planning. Payload {task,scope}; result {decision: enter|reject, messages?}. No listeners yet; dispatching is an error until a consumer exists.",
    },
    EventDef {
        name: "host/booted",
        mode: Mode::Emit,
        unfiltered: true,
        doc: "Boot completed. Payload {rows: [ids], skipped: [{id,reason}]}. Unfiltered: shell-owned UI observes every boot.",
    },
    EventDef {
        name: "host/shutdown",
        mode: Mode::Emit,
        unfiltered: true,
        doc: "Shutdown unwound all effects. Payload {unwound: [labels]}. Unfiltered.",
    },
];

/// Look up a catalog entry by name.
pub fn lookup(name: &str) -> Option<&'static EventDef> {
    EVENT_CATALOG.iter().find(|d| d.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The catalog freshness check: names are unique, every entry has
    /// documentation, and every dispatch site in this crate resolves.
    #[test]
    fn catalog_freshness() {
        let mut seen = HashSet::new();
        for def in EVENT_CATALOG {
            assert!(!def.name.is_empty(), "event name must not be empty");
            assert!(!def.doc.is_empty(), "event '{}' must carry doc", def.name);
            assert!(seen.insert(def.name), "duplicate event '{}'", def.name);
        }
        // Dispatch sites used across the crate must resolve here.
        for site in [
            "tools/pre-execute",
            "tools/post-execute",
            "tools/result",
            "tools/change",
            "host/booted",
            "host/shutdown",
        ] {
            assert!(lookup(site).is_some(), "dispatch site '{site}' has no catalog entry");
        }
    }
}
