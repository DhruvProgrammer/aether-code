//! Basic tri-state verdict + v0.11 backward-compat `Policy`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Permission {
    Allow,
    Deny,
    Ask,
}

impl Permission {
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "allow" => Self::Allow,
            "deny" => Self::Deny,
            _ => Self::Ask,
        }
    }

    pub fn is_allow(self) -> bool { matches!(self, Self::Allow) }
    pub fn is_deny(self) -> bool { matches!(self, Self::Deny) }
    pub fn is_ask(self) -> bool { matches!(self, Self::Ask) }
}

/// Backward-compatible flat policy. Exposed so v0.11 callers still work.
#[derive(Debug, Clone)]
pub struct Policy {
    pub read: Permission,
    pub edit: Permission,
    pub bash: Permission,
    pub delete: Permission,
    pub git_commit: Permission,
    pub network: Permission,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            read: Permission::Allow,
            edit: Permission::Allow,
            bash: Permission::Ask,
            delete: Permission::Ask,
            git_commit: Permission::Ask,
            network: Permission::Ask,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn parse_allow() { assert_eq!(Permission::parse("allow"), Permission::Allow); }
    #[test] fn parse_deny() { assert_eq!(Permission::parse("DENY"), Permission::Deny); }
    #[test] fn parse_unknown_is_ask() { assert_eq!(Permission::parse("whatever"), Permission::Ask); }
}
