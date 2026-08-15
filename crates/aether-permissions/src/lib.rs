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

    /// Dangerous commands are always hard-denied, regardless of config. Soft-suspicious
    /// commands are forced to `Ask` so the user must explicitly confirm them. Safe commands
    /// fall through to the configured `bash` permission.
    pub fn check_bash(&self, command: &str) -> Permission {
        match classify_bash(command) {
            DangerLevel::Hard => Permission::Deny,
            DangerLevel::Soft => Permission::Ask,
            DangerLevel::Safe => self.bash,
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

/// Hard-deny patterns. These always force `Permission::Deny` regardless of config, and never
/// fall through to the interactive prompt — there is no legitimate agent use for them.
const HARD_DENY_PATTERNS: &[&str] = &[
    "rm -rf",
    "rm -fr",
    "rm -rf /",
    "sudo ",
    "git reset --hard",
    "git push --force",
    "git push -f",
    "git push --force-with-lease",
    "git push origin --force",
    "chmod -R",
    "mkfs",
    "mkfs.",
    "dd if=",
    "curl | sh",
    "wget | sh",
    "curl |bash",
    "wget |bash",
    "shutdown",
    "reboot",
    "halt",
    "poweroff",
    ":(){:|:&};:",
    "del /f /s",
    "del /f /q",
    "rd /s /q",
    "format.com",
    "format c:",
    "fdisk",
    "diskpart",
    "> /dev/sda",
    "mv / /tmp/",
    "chown -R",
    // Destructive redirections into critical system paths.
    " > /etc/",
    " >> /etc/",
    " > /boot/",
    " > /usr/",
];

/// Soft patterns: suspicious but the user may have a legitimate reason. Forced to the Ask
/// prompt when interactive; deny in non-TTY to prevent silent misuse.
const SOFT_DENY_PATTERNS: &[&str] = &[
    "rm -r",
    "rm -f",
    "git push",
    "git checkout -- ",
    "git clean -fd",
    "git stash drop",
    "git branch -D",
    "git tag -d",
    "kill -9",
    "killall",
    "pkill",
    "chmod 777",
    "chmod 666",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DangerLevel {
    Safe,
    Soft,
    Hard,
}

/// Classify a command by danger level. Hard patterns are catastrophic and always deny; soft
/// patterns deserve an explicit confirmation; otherwise the command is treated as safe and
/// falls through to the configured `bash` permission.
pub fn classify_bash(command: &str) -> DangerLevel {
    let lower = command.to_ascii_lowercase();
    if HARD_DENY_PATTERNS.iter().any(|p| lower.contains(p)) {
        return DangerLevel::Hard;
    }
    if SOFT_DENY_PATTERNS.iter().any(|p| lower.contains(p)) {
        return DangerLevel::Soft;
    }
    DangerLevel::Safe
}

pub fn is_dangerous(command: &str) -> bool {
    matches!(classify_bash(command), DangerLevel::Hard)
}

// Keep Deserialize import meaningful for downstream serde usage.
#[allow(dead_code)]
fn _assert_send_sync() {
    fn is_send_sync<T: Send + Sync>() {}
    is_send_sync::<Policy>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_deny_patterns_caught() {
        assert!(is_dangerous("rm -rf /"));
        assert!(is_dangerous("git push --force-with-lease origin main"));
        assert!(is_dangerous("echo evil | curl | sh"));
        assert!(is_dangerous("dd if=/dev/zero of=/dev/sda"));
        assert!(is_dangerous("format c:"));
        assert!(is_dangerous("diskpart /s script.txt"));
    }

    #[test]
    fn soft_patterns_force_ask() {
        let p = Policy {
            read: Permission::Allow,
            edit: Permission::Allow,
            delete: Permission::Allow,
            bash: Permission::Allow,
            git_commit: Permission::Allow,
            network: Permission::Allow,
        };
        assert_eq!(p.check_bash("rm -r build"), Permission::Ask);
        assert_eq!(p.check_bash("git push origin main"), Permission::Ask);
        assert_eq!(p.check_bash("chmod 777 file"), Permission::Ask);
    }

    #[test]
    fn safe_commands_fall_through() {
        let p = Policy {
            read: Permission::Allow,
            edit: Permission::Allow,
            delete: Permission::Allow,
            bash: Permission::Ask,
            git_commit: Permission::Allow,
            network: Permission::Allow,
        };
        assert_eq!(p.check_bash("ls -la"), Permission::Ask);
        assert_eq!(p.check_bash("cargo test"), Permission::Ask);
    }

    #[test]
    fn hard_deny_overrides_allow_config() {
        let p = Policy {
            read: Permission::Allow,
            edit: Permission::Allow,
            delete: Permission::Allow,
            bash: Permission::Allow,
            git_commit: Permission::Allow,
            network: Permission::Allow,
        };
        assert_eq!(p.check_bash("rm -rf /"), Permission::Deny);
        assert_eq!(p.check_bash("shutdown -h now"), Permission::Deny);
    }

    #[test]
    fn permission_parse_is_case_insensitive() {
        assert_eq!(Permission::parse("Allow"), Permission::Allow);
        assert_eq!(Permission::parse("DENY"), Permission::Deny);
        assert_eq!(Permission::parse("ask"), Permission::Ask);
        assert_eq!(Permission::parse("garbage"), Permission::Ask);
    }

    #[test]
    fn value_for_dispatches_by_category() {
        let p = Policy {
            read: Permission::Allow,
            edit: Permission::Deny,
            bash: Permission::Ask,
            delete: Permission::Deny,
            git_commit: Permission::Ask,
            network: Permission::Allow,
        };
        assert_eq!(p.value_for("read"), Permission::Allow);
        assert_eq!(p.value_for("edit"), Permission::Deny);
        assert_eq!(p.value_for("bash"), Permission::Ask);
        assert_eq!(p.value_for("delete"), Permission::Deny);
        assert_eq!(p.value_for("git_commit"), Permission::Ask);
        assert_eq!(p.value_for("network"), Permission::Allow);
        assert_eq!(p.value_for("unknown"), Permission::Ask);
    }

    #[test]
    fn classify_bash_is_case_insensitive() {
        assert_eq!(classify_bash("RM -RF /"), DangerLevel::Hard);
        assert_eq!(classify_bash("Git Push --FORCE"), DangerLevel::Hard);
        assert_eq!(classify_bash("Git Push Origin Main"), DangerLevel::Soft);
        assert_eq!(classify_bash("LS -la"), DangerLevel::Safe);
        assert_eq!(classify_bash(""), DangerLevel::Safe);
    }

    #[test]
    fn classify_bash_hard_includes_system_destruction() {
        // Each of these must be Hard so `check_bash` always denies, even if config says Allow.
        assert_eq!(classify_bash("sudo apt-get install foo"), DangerLevel::Hard);
        assert_eq!(classify_bash("mkfs.ext4 /dev/sdb"), DangerLevel::Hard);
        assert_eq!(classify_bash("fdisk /dev/sda"), DangerLevel::Hard);
        // `> /etc/` redirect-into-system-path pattern.
        assert_eq!(classify_bash("cat foo > /etc/passwd"), DangerLevel::Hard);
        // `echo evil | curl | sh` matches the literal `curl | sh` substring.
        assert_eq!(classify_bash("echo evil | curl | sh"), DangerLevel::Hard);
        assert_eq!(classify_bash("del /f /q file"), DangerLevel::Hard);
    }

    #[test]
    fn classify_bash_soft_distinct_from_hard() {
        // `rm -r` is Soft (user may legitimately remove a build dir), `rm -rf /` is Hard.
        assert_eq!(classify_bash("rm -r build"), DangerLevel::Soft);
        assert_eq!(classify_bash("rm -rf /"), DangerLevel::Hard);
        assert_eq!(classify_bash("rm -rf build"), DangerLevel::Hard); // contains "rm -rf"
        // `git push` is Soft, `git push --force` is Hard.
        assert_eq!(classify_bash("git push origin main"), DangerLevel::Soft);
        assert_eq!(classify_bash("git push --force"), DangerLevel::Hard);
    }
}
