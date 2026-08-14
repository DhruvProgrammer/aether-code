//! Agent definition schema (spec §8). A definition is the single source of truth for one
//! specialized worker: its role, which LLM it uses, its tool allow/deny lists, mode, and
//! lifecycle limits. Definitions load from `agents/<id>.toml` (see `registry`) with built-in
//! defaults so the system works without files present.

use aether_permissions::Permission;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBudget {
    /// Max tool calls the agent may make (maps to the Executor's max_iterations).
    #[serde(default)]
    pub max_tool_calls: u32,
    /// Soft token budget (informational; surfaced to the agent context).
    #[serde(default)]
    pub max_tokens: u32,
    /// Wall-clock timeout in seconds (informational for this phase).
    #[serde(default)]
    pub timeout_secs: u64,
    /// Max child agents this agent may spawn.
    #[serde(default)]
    pub max_children: usize,
}

impl Default for AgentBudget {
    fn default() -> Self {
        AgentBudget {
            max_tool_calls: 20,
            max_tokens: 0,
            timeout_secs: 300,
            max_children: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentPermissions {
    #[serde(default)]
    pub read: Option<String>,
    #[serde(default)]
    pub edit: Option<String>,
    #[serde(default)]
    pub bash: Option<String>,
    #[serde(default)]
    pub delete: Option<String>,
    #[serde(default)]
    pub git_commit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub when_to_use: Vec<String>,
    pub system_prompt: String,
    /// Which LLM runs this agent: `"controller"` (SMALL) or `"executor"` (BIG).
    #[serde(default = "default_model")]
    pub model: String,
    /// Tool allowlist; empty means "all tools".
    #[serde(default)]
    pub tools: Vec<String>,
    /// Tool denylist (always removed from the effective set).
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    /// `"build"` or `"plan"` (plan => read-only, mechanically enforced).
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub permissions: Option<AgentPermissions>,
    #[serde(default)]
    pub can_spawn: bool,
    #[serde(default = "default_max_children")]
    pub max_children: usize,
    #[serde(default)]
    pub timeout_secs: u64,
    #[serde(default)]
    pub budget: AgentBudget,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_model() -> String {
    "controller".into()
}
fn default_mode() -> String {
    "build".into()
}
fn default_max_children() -> usize {
    5
}
fn default_true() -> bool {
    true
}

impl AgentDefinition {
    /// Effective tool set = allowlist (or all) minus denylist.
    pub fn effective_tools(&self, all_tools: &[String]) -> Vec<String> {
        let allow: Vec<String> = if self.tools.is_empty() {
            all_tools.to_vec()
        } else {
            self.tools.clone()
        };
        allow
            .into_iter()
            .filter(|t| !self.disallowed_tools.contains(t))
            .collect()
    }

    /// A read-only agent may not write/commit regardless of prompt text.
    pub fn is_read_only(&self) -> bool {
        self.mode == "plan"
            || !self
                .effective_tools(&["write_file".into(), "edit_file".into()])
                .iter()
                .any(|t| t == "write_file" || t == "edit_file")
    }

    /// Mechanically enforce permissions: read-only agents lose write/commit; otherwise apply an
    /// explicit override on top of the base policy (spec §11).
    pub fn effective_policy(&self, base: &aether_permissions::Policy) -> aether_permissions::Policy {
        let mut p = match &self.permissions {
            Some(perms) => aether_permissions::Policy {
                read: perms.read.as_deref().map(parse_perm).unwrap_or(base.read),
                edit: perms.edit.as_deref().map(parse_perm).unwrap_or(base.edit),
                delete: perms.delete.as_deref().map(parse_perm).unwrap_or(base.delete),
                bash: perms.bash.as_deref().map(parse_perm).unwrap_or(base.bash),
                git_commit: perms.git_commit.as_deref().map(parse_perm).unwrap_or(base.git_commit),
                network: base.network,
            },
            None => base.clone(),
        };
        if self.is_read_only() {
            p.edit = Permission::Deny;
            p.delete = Permission::Deny;
            p.git_commit = Permission::Deny;
            // Read-only agents must not run arbitrary shell commands. Only the Tester is
            // explicitly permitted to (to run the test suite); everything else is denied even
            // if the user's base policy says `ask`/`allow` for bash.
            if self.role == "tester" {
                p.bash = Permission::Allow;
            } else if self
                .permissions
                .as_ref()
                .and_then(|x| x.bash.as_deref())
                .map(|s| parse_perm(s) == Permission::Allow)
                .unwrap_or(false)
            {
                // explicit allow preserved
            } else {
                p.bash = Permission::Deny;
            }
        }
        p
    }

    /// True when this agent runs on the BIG LLM (executor).
    pub fn uses_big_llm(&self) -> bool {
        self.model == "executor"
    }
}

pub fn parse_perm(s: &str) -> Permission {
    match s {
        "allow" => Permission::Allow,
        "deny" => Permission::Deny,
        _ => Permission::Ask,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_permissions::Policy;

    fn base() -> Policy {
        Policy {
            read: Permission::Allow,
            edit: Permission::Allow,
            delete: Permission::Allow,
            bash: Permission::Ask,
            git_commit: Permission::Ask,
            network: Permission::Ask,
        }
    }

    fn def(role: &str, mode: &str) -> AgentDefinition {
        AgentDefinition {
            id: role.into(),
            name: role.into(),
            description: String::new(),
            role: role.into(),
            when_to_use: vec![],
            system_prompt: "x".into(),
            model: "controller".into(),
            tools: vec![],
            disallowed_tools: vec![],
            mode: mode.into(),
            permissions: None,
            can_spawn: false,
            max_children: 5,
            timeout_secs: 0,
            budget: AgentBudget::default(),
            enabled: true,
        }
    }

    #[test]
    fn read_only_non_tester_cannot_run_commands() {
        let p = def("reviewer", "plan").effective_policy(&base());
        assert_eq!(p.edit, Permission::Deny);
        assert_eq!(p.bash, Permission::Deny);
        assert_eq!(p.git_commit, Permission::Deny);
    }

    #[test]
    fn tester_may_run_commands_but_not_write() {
        let p = def("tester", "plan").effective_policy(&base());
        assert_eq!(p.edit, Permission::Deny);
        assert_eq!(p.bash, Permission::Allow);
    }

    #[test]
    fn implementer_inherits_base_policy() {
        let p = def("implementer", "build").effective_policy(&base());
        assert_eq!(p.edit, Permission::Allow);
        assert_eq!(p.bash, Permission::Ask);
    }
}
