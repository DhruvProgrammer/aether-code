//! Agent registry (spec §9): load/validate/list/find/enable/disable agent definitions.
//! Built-in defaults cover the required 10 agents; `agents/<id>.toml` files override them.

use std::collections::HashMap;
use std::path::Path;

use crate::agents::definition::AgentDefinition;

fn agent(
    id: &str,
    name: &str,
    role: &str,
    model: &str,
    mode: &str,
    system_prompt: &str,
    when_to_use: &[&str],
) -> AgentDefinition {
    AgentDefinition {
        id: id.into(),
        name: name.into(),
        description: name.into(),
        role: role.into(),
        when_to_use: when_to_use.iter().map(|s| s.to_string()).collect(),
        system_prompt: system_prompt.into(),
        model: model.into(),
        tools: vec![],
        disallowed_tools: vec![],
        mode: mode.into(),
        permissions: None,
        can_spawn: false,
        max_children: 5,
        timeout_secs: 300,
        budget: Default::default(),
        enabled: true,
    }
}

fn builtins() -> Vec<AgentDefinition> {
    vec![
        agent(
            "planner",
            "Planner",
            "planner",
            "controller",
            "plan",
            "You are the Planner (SMALL LLM). Convert the request into an implementation-ready plan. \
             Inspect context, identify scope/dependencies/unknowns/risks, define steps and verification. \
             Do NOT modify source. Output a concise plan. Apply Karpathy: simplest viable approach.",
            &["plan", "implement", "design", "architecture", "steps"],
        ),
        agent(
            "designer",
            "Designer",
            "designer",
            "controller",
            "plan",
            "You are the Designer (SMALL LLM). Determine the simplest appropriate technical design. \
             Identify reusable abstractions, affected modules, interfaces, data flow; compare alternatives \
             and tradeoffs; recommend the minimal design. Do NOT modify source. Apply Karpathy: do not over-engineer.",
            &["design", "architecture", "interface", "abstraction", "refactor"],
        ),
        agent(
            "explorer",
            "Explorer",
            "explorer",
            "controller",
            "plan",
            "You are the Explorer (SMALL LLM). Map the repository read-only: locate relevant files, symbols, \
             dependencies, entry points, tests, configuration, and Git history. Report findings concisely. \
             Never modify anything.",
            &["find", "locate", "where", "investigate", "search", "understand", "explore"],
        ),
        agent(
            "researcher",
            "Researcher",
            "researcher",
            "controller",
            "plan",
            "You are the Researcher (SMALL LLM). Answer a focused technical question using available docs, \
             memory, and skills. Return findings, sources, confidence, and a recommendation. Stay focused.",
            &["research", "documentation", "api", "how does", "what is", "best practice"],
        ),
        agent(
            "implementer",
            "Implementer",
            "implementer",
            "executor",
            "build",
            "You are the Implementer (BIG LLM). Implement the structured task using the available tools. \
             Make surgical changes; match existing style; do not over-engineer. When done, reply with a \
             final summary and no tool calls.",
            &["implement", "code", "write", "fix", "build", "refactor", "create"],
        ),
        agent(
            "tester",
            "Tester",
            "tester",
            "controller",
            "plan",
            "You are the Tester (SMALL LLM). Run the project's tests and interpret results. Distinguish \
             'test passed' from 'user requirement verified'. You may run read-only test commands. Return JSON \
             {\"status\":\"ok\"|\"failed\",\"summary\":string,\"findings\":[string]}.",
            &["test", "run tests", "verify", "regression", "passing"],
        ),
        agent(
            "reviewer",
            "Reviewer",
            "reviewer",
            "controller",
            "plan",
            "You are the Reviewer (SMALL LLM). Independently inspect the actual diff, requirements, tests, \
             architecture, and scope. Do not trust 'done' blindly. Return JSON \
             {\"status\":\"ok\"|\"changes_requested\",\"summary\":string,\"findings\":[string]}.",
            &["review", "check", "audit", "quality", "maintainability"],
        ),
        agent(
            "debugger",
            "Debugger",
            "debugger",
            "controller",
            "plan",
            "You are the Debugger (SMALL LLM). Given facts/hypotheses/evidence, determine WHY something \
             failed and what should happen next. Use evidence, not guesses. Do not blindly retry. Return a \
             concise diagnosis and recommended next step.",
            &["debug", "why", "failed", "error", "broken", "investigate failure"],
        ),
        agent(
            "security-reviewer",
            "Security Reviewer",
            "security-reviewer",
            "controller",
            "plan",
            "You are the Security Reviewer (SMALL LLM). Review for input validation, secret handling, \
             injection (command/path/SQL), authn/authz, permissions, and dependency risk. Return JSON \
             {\"status\":\"ok\"|\"changes_requested\",\"summary\":string,\"findings\":[string]}.",
            &["security", "auth", "secret", "injection", "permission", "vulnerab", "owasp"],
        ),
        agent(
            "documenter",
            "Documenter",
            "documenter",
            "controller",
            "plan",
            "You are the Documenter (SMALL LLM). Update only relevant documentation (README, API docs, \
             changelog, focused comments). Do not rewrite unrelated docs. Return a concise summary of changes.",
            &["document", "readme", "docs", "changelog", "comment"],
        ),
    ]
}

#[derive(Debug, Default)]
pub struct AgentRegistry {
    agents: HashMap<String, AgentDefinition>,
}

impl AgentRegistry {
    /// Built-in defaults (all 10 agents), ignoring any on-disk overrides.
    pub fn builtin() -> Self {
        let mut m = HashMap::new();
        for a in builtins() {
            m.insert(a.id.clone(), a);
        }
        Self { agents: m }
    }

    /// Built-in defaults, then override with any `agents/<id>.toml` found under `dir`.
    pub fn load_from_dir(dir: &Path) -> Self {
        let mut reg = Self::builtin();
        let agents_dir = dir.join("agents");
        if let Ok(entries) = std::fs::read_dir(&agents_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        if let Ok(def) = toml::from_str::<AgentDefinition>(&text) {
                            reg.agents.insert(def.id.clone(), def);
                        }
                    }
                }
            }
        }
        reg
    }

    pub fn register(&mut self, def: AgentDefinition) {
        self.agents.insert(def.id.clone(), def);
    }

    pub fn find(&self, id: &str) -> Option<&AgentDefinition> {
        self.agents.get(id)
    }

    pub fn list(&self) -> Vec<&AgentDefinition> {
        let mut v: Vec<&AgentDefinition> = self.agents.values().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    pub fn enabled(&self) -> Vec<&AgentDefinition> {
        self.list().into_iter().filter(|a| a.enabled).collect()
    }

    pub fn enable(&mut self, id: &str) {
        if let Some(a) = self.agents.get_mut(id) {
            a.enabled = true;
        }
    }

    pub fn disable(&mut self, id: &str) {
        if let Some(a) = self.agents.get_mut(id) {
            a.enabled = false;
        }
    }

    /// Validation errors (empty == valid).
    pub fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();
        for a in self.agents.values() {
            if a.id.trim().is_empty() {
                errs.push("agent with empty id".into());
            }
            if a.system_prompt.trim().is_empty() {
                errs.push(format!("agent '{}' has empty system_prompt", a.id));
            }
            if a.model != "controller" && a.model != "executor" {
                errs.push(format!(
                    "agent '{}' model must be 'controller' or 'executor' (got '{}')",
                    a.id, a.model
                ));
            }
            if a.mode != "build" && a.mode != "plan" {
                errs.push(format!(
                    "agent '{}' mode must be 'build' or 'plan' (got '{}')",
                    a.id, a.mode
                ));
            }
        }
        errs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_valid() {
        let reg = AgentRegistry::builtin();
        assert_eq!(reg.list().len(), 10);
        assert!(reg.validate().is_empty(), "{:?}", reg.validate());
    }

    #[test]
    fn enable_disable() {
        let mut reg = AgentRegistry::builtin();
        reg.disable("tester");
        assert!(!reg.find("tester").unwrap().enabled);
        reg.enable("tester");
        assert!(reg.find("tester").unwrap().enabled);
    }

    #[test]
    fn load_overrides_builtin() {
        let tmp = std::env::temp_dir().join(format!("aether-agents-{}", std::process::id()));
        let _ = std::fs::create_dir_all(tmp.join("agents"));
        let path = tmp.join("agents/explorer.toml");
        std::fs::write(
            &path,
            "id = \"explorer\"\nname = \"Explorer\"\ndescription = \"x\"\nrole = \"explorer\"\nmodel = \"controller\"\nmode = \"plan\"\nsystem_prompt = \"override\"\n",
        )
        .unwrap();
        let reg = AgentRegistry::load_from_dir(&tmp);
        assert_eq!(reg.find("explorer").unwrap().system_prompt, "override");
    }
}
