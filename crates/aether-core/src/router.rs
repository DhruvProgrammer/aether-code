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

#[cfg(test)]
mod tests {
    use super::*;

    fn route(task: &str) -> String {
        select_model(task, Some("fast"), "executor", "controller")
    }

    #[test]
    fn read_only_short_goes_cheap() {
        assert_eq!(route("explain the diff"), "fast");
        assert_eq!(route("what is the main loop?"), "fast");
        assert_eq!(route("find the parser bug"), "fast");
    }

    #[test]
    fn implements_goes_capable() {
        assert_eq!(route("implement the new endpoint"), "executor");
        assert_eq!(route("fix the parser bug in main.rs"), "executor");
        assert_eq!(route("refactor the agent loop"), "executor");
        assert_eq!(route("build a beautiful landing page"), "executor");
    }

    #[test]
    fn long_read_only_without_implement_keyword_goes_capable() {
        // The router routes long read-only tasks (>= 120 chars) to the capable model even
        // without an implement keyword, to avoid under-thinking a complex question.
        let task = "explain how the visual engineering loop, the controller, the executor, \
                    and the temporary screenshot manager interact across iterations.";
        assert_eq!(route(task), "executor");
    }

    #[test]
    fn medium_read_only_still_cheap() {
        // Under 120 chars + read-only intent → cheap.
        let task = "explain how the visual loop reaches the acceptance policy and escalates";
        assert!(task.len() < 120);
        assert_eq!(route(task), "fast");
    }

    #[test]
    fn ambiguous_long_task_defaults_capable() {
        let task = "consider the tradeoffs of the new feature and propose a plan for how to \
                    integrate it with the existing module without breaking anything.";
        assert_eq!(route(task), "executor");
    }

    #[test]
    fn cheap_falls_back_to_controller_when_none() {
        assert_eq!(
            select_model("explain the diff", None, "executor", "controller"),
            "controller"
        );
    }
}
