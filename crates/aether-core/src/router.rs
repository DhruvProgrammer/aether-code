//! Cost-aware model routing (spec §8). Heuristic, no classifier needed: trivial/read-only
//! intents route to a cheaper model; implementation intents go to the capable model.

/// Pick the executor model for a given task.
/// `cheap` falls back to `controller`; `capable` is the configured executor model.
pub fn select_model(task: &str, cheap: Option<&str>, capable: &str, controller: &str) -> String {
    let cheap_key = cheap.unwrap_or(controller);
    let t = task.trim();
    let lower = t.to_ascii_lowercase();

    // Read-only / explanatory intents → cheap.
    let read_only = lower.starts_with("what")
        || lower.starts_with("why")
        || lower.starts_with("how")
        || lower.starts_with("explain")
        || lower.starts_with("describe")
        || lower.starts_with("show")
        || lower.starts_with("list")
        || lower.starts_with("find")
        || lower.contains("what is")
        || lower.contains("how do")
        || lower.contains("explain");

    // Trivial length → cheap.
    if t.len() < 80 && read_only {
        return cheap_key.to_string();
    }

    // Implementation intents → capable.
    let implements = lower.contains("implement")
        || lower.contains("fix")
        || lower.contains("refactor")
        || lower.contains("write")
        || lower.contains("create")
        || lower.contains("add ")
        || lower.contains("build")
        || lower.contains("change");

    if implements {
        return capable.to_string();
    }

    // Default: capable for anything non-trivial, else cheap.
    if t.len() < 120 && read_only {
        cheap_key.to_string()
    } else {
        capable.to_string()
    }
}
