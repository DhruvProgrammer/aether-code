//! Agent router (spec §44). Chooses which agents to run for a task and which LLM each uses.
//! Model routing is enforced later by `Agent::resolve_provider` (Implementer -> BIG, others -> SMALL).

use crate::agents::registry::AgentRegistry;

pub struct AgentRouter;

impl AgentRouter {
    /// Rank enabled agents by relevance to `task` (keyword overlap on `when_to_use`/`role`/`id`).
    /// Returns agent ids best-first. The Controller may pick none ("no agent needed", spec §45).
    pub fn route(registry: &AgentRegistry, task: &str) -> Vec<String> {
        let t = task.to_lowercase();
        if t.trim().is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(usize, String)> = registry
            .enabled()
            .iter()
            .map(|a| {
                let mut score = 0usize;
                for kw in &a.when_to_use {
                    if t.contains(&kw.to_lowercase()) {
                        score += 2;
                    }
                }
                if t.contains(&a.role.to_lowercase()) {
                    score += 2;
                }
                if t.contains(&a.id.to_lowercase()) {
                    score += 3;
                }
                (score, a.id.clone())
            })
            .filter(|(s, _)| *s > 0)
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter().map(|(_, id)| id).collect()
    }

    /// Verification pipeline run after the BIG LLM implements (spec §17-§18, §58).
    /// Always Tester + Reviewer; Security Reviewer added on high risk or security-related tasks.
    pub fn select_verification(registry: &AgentRegistry, task: &str, high_risk: bool) -> Vec<String> {
        let mut out = Vec::new();
        let sec_kw = ["security", "auth", "secret", "injection", "permission", "vulnerab", "owasp", "token", "password"];
        let looks_security = sec_kw.iter().any(|k| task.to_lowercase().contains(k));
        for id in ["tester", "reviewer"] {
            if registry.find(id).map(|a| a.enabled).unwrap_or(false) {
                out.push(id.to_string());
            }
        }
        if (high_risk || looks_security)
            && registry.find("security-reviewer").map(|a| a.enabled).unwrap_or(false)
        {
            out.push("security-reviewer".to_string());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_by_keywords() {
        let reg = AgentRegistry::builtin();
        let r = AgentRouter::route(&reg, "debug why the login fails");
        assert!(r.contains(&"debugger".to_string()));
    }

    #[test]
    fn empty_task_no_agents() {
        let reg = AgentRegistry::builtin();
        assert!(AgentRouter::route(&reg, "   ").is_empty());
    }

    #[test]
    fn verification_always_has_tester_reviewer() {
        let reg = AgentRegistry::builtin();
        let v = AgentRouter::select_verification(&reg, "implement oauth", false);
        assert!(v.contains(&"tester".to_string()));
        assert!(v.contains(&"reviewer".to_string()));
    }

    #[test]
    fn security_task_adds_security_reviewer() {
        let reg = AgentRegistry::builtin();
        let v = AgentRouter::select_verification(&reg, "fix auth token injection", false);
        assert!(v.contains(&"security-reviewer".to_string()));
    }
}
