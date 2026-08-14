//! Permission policy engine (spec §14).

use aether_config::PermissionsConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Allow,
    Deny,
    Ask,
}

impl Permission {
    pub fn parse(s: &str) -> Permission {
        match s.to_ascii_lowercase().as_str() {
            "allow" => Permission::Allow,
            "deny" => Permission::Deny,
            _ => Permission::Ask,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Policy {
    pub read: Permission,
    pub edit: Permission,
    pub bash: Permission,
    pub delete: Permission,
    pub git_commit: Permission,
    pub network: Permission,
}

impl Policy {
    pub fn from_config(c: &PermissionsConfig) -> Policy {
        Policy {
            read: Permission::parse(&c.read),
            edit: Permission::parse(&c.edit),
            bash: Permission::parse(&c.bash),
            delete: Permission::parse(&c.delete),
            git_commit: Permission::parse(&c.git_commit),
            network: Permission::parse(&c.network),
        }
    }

    /// Dangerous commands are always hard-denied, regardless of config. Less dangerous
    /// commands fall through to the configured `bash` permission.
    pub fn check_bash(&self, command: &str) -> Permission {
        if is_dangerous(command) {
            Permission::Deny
        } else {
            self.bash
        }
    }

    /// Resolve the policy value for a tool's category (spec §14).
    pub fn value_for(&self, category: &str) -> Permission {
        match category {
            "read" => self.read,
            "edit" => self.edit,
            "bash" => self.bash,
            "delete" => self.delete,
            "git_commit" => self.git_commit,
            "network" => self.network,
            _ => Permission::Ask,
        }
    }
}

pub fn is_dangerous(command: &str) -> bool {
    const PATTERNS: &[&str] = &[
        "rm -rf",
        "sudo ",
        "git reset --hard",
        "git push --force",
        "git push -f",
        "chmod -R",
        "mkfs",
        "dd if=",
    ];
    let lower = command.to_ascii_lowercase();
    PATTERNS.iter().any(|p| lower.contains(p))
}

// Keep Deserialize import meaningful for downstream serde usage.
#[allow(dead_code)]
fn _assert_send_sync() {
    fn is_send_sync<T: Send + Sync>() {}
    is_send_sync::<Policy>();
}
