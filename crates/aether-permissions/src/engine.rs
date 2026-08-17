//! The hierarchical permission engine.
//!
//! `PermissionEngine` owns a stack of rule lists (Global → Project → Role →
//! Agent → Tool). When asked to decide an `(operation, scope)`, it walks the
//! stack in priority order. The first matching rule wins; Deny beats Allow
//! across layers (defense in depth). When no rule matches the engine falls
//! back to the default `Permission::Ask`.

use super::approval::{ApprovalChannel, ApprovalRequest, ApprovalResponse, ApprovalScope, DenyAllChannel};
use super::decision::{DecisionLog, DecisionRecord, InMemorySink, PermissionEventSink};
use super::policy::{Permission, Policy};
use super::rule::{Rule, RuleMatch, RuleSource};
use super::scope::{Operation, ResourceScope};
use chrono::Utc;
use std::sync::{Arc, Mutex};

/// Engine configuration.
#[derive(Clone)]
pub struct PermissionEngine {
    inner: Arc<Inner>,
}

struct Inner {
    global: Mutex<Vec<Rule>>,
    project: Mutex<Vec<Rule>>,
    roles: Mutex<std::collections::HashMap<String, Vec<Rule>>>,
    agents: Mutex<std::collections::HashMap<String, Vec<Rule>>>,
    tools: Mutex<std::collections::HashMap<String, Vec<Rule>>>,
    /// Session-scoped grants accumulated at runtime (Allow-for-session).
    session_grants: Mutex<Vec<Rule>>,
    /// Default permission if no rule matches.
    default: Mutex<Permission>,
    log: DecisionLog,
    sink: Mutex<Arc<dyn PermissionEventSink>>,
    approval: Mutex<Arc<dyn ApprovalChannel>>,
}

impl std::fmt::Debug for PermissionEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionEngine").field("default", &self.inner.default).finish()
    }
}

impl Default for PermissionEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionEngine {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                global: Mutex::new(Vec::new()),
                project: Mutex::new(Vec::new()),
                roles: Mutex::new(Default::default()),
                agents: Mutex::new(Default::default()),
                tools: Mutex::new(Default::default()),
                session_grants: Mutex::new(Vec::new()),
                default: Mutex::new(Permission::Ask),
                log: DecisionLog::new(),
                sink: Mutex::new(Arc::new(InMemorySink::default())),
                approval: Mutex::new(Arc::new(DenyAllChannel)),
            }),
        }
    }

    pub fn with_default(self, p: Permission) -> Self { *self.inner.default.lock().unwrap() = p; self }
    pub fn with_sink(self, sink: Arc<dyn PermissionEventSink>) -> Self { *self.inner.sink.lock().unwrap() = sink; self }
    pub fn with_approval(self, ch: Arc<dyn ApprovalChannel>) -> Self { *self.inner.approval.lock().unwrap() = ch; self }

    pub fn from_policy(p: &Policy) -> Self {
        let engine = Self::new().with_default(Permission::Ask);
        engine.push_global(p.to_rules());
        engine
    }

    pub fn log(&self) -> &DecisionLog { &self.inner.log }

    pub fn add_global(&self, r: Rule) { self.inner.global.lock().unwrap().push(r); }
    pub fn add_project(&self, r: Rule) { self.inner.project.lock().unwrap().push(r); }
    pub fn add_role(&self, role: impl Into<String>, r: Rule) {
        self.inner.roles.lock().unwrap().entry(role.into()).or_default().push(r);
    }
    pub fn add_agent(&self, agent: impl Into<String>, r: Rule) {
        self.inner.agents.lock().unwrap().entry(agent.into()).or_default().push(r);
    }
    pub fn add_tool(&self, tool: impl Into<String>, r: Rule) {
        self.inner.tools.lock().unwrap().entry(tool.into()).or_default().push(r);
    }

    pub fn push_global(&self, rs: Vec<Rule>) { for r in rs { self.add_global(r); } }
    pub fn push_project(&self, rs: Vec<Rule>) { for r in rs { self.add_project(r); } }

    /// Record a session-scoped grant (e.g. user clicked "Allow for session").
    pub fn grant_session(&self, r: Rule) { self.inner.session_grants.lock().unwrap().push(r); }

    pub fn set_default(&self, p: Permission) { *self.inner.default.lock().unwrap() = p; }

    pub fn set_approval(&self, ch: Arc<dyn ApprovalChannel>) { *self.inner.approval.lock().unwrap() = ch; }

    /// Decide an operation.
    pub fn decide(
        &self,
        op: Operation,
        scope: &ResourceScope,
        ctx: DecisionContext<'_>,
    ) -> DecisionRecord {
        let lookup = |source: RuleSource, rules: &[Rule]| -> Option<RuleMatch> {
            for r in rules {
                if r.matches(op, scope) { return Some(RuleMatch { rule: r.clone(), source }); }
            }
            None
        };

        // Priority: Session > Agent > Tool > Role > Project > Global.
        // (Inline / Session comes last so that user "Allow for session" grants
        //  do not override the engine's deny rules.)
        let _ignored_layers_marker: Option<()> = None;
        let session = self.inner.session_grants.lock().unwrap().clone();
        let agents = ctx.agent_id.and_then(|id| self.inner.agents.lock().unwrap().get(id).cloned()).unwrap_or_default();
        let tools = ctx.tool.and_then(|t| self.inner.tools.lock().unwrap().get(t).cloned()).unwrap_or_default();
        let roles = ctx.role.and_then(|r| self.inner.roles.lock().unwrap().get(r).cloned()).unwrap_or_default();
        let project = self.inner.project.lock().unwrap().clone();
        let global = self.inner.global.lock().unwrap().clone();

        let candidates: [(RuleSource, Vec<Rule>); 6] = [
            (RuleSource::Agent, agents),
            (RuleSource::Tool, tools),
            (RuleSource::Role, roles),
            (RuleSource::Project, project),
            (RuleSource::Global, global),
            (RuleSource::Inline, session),
        ];

        let mut verdict: Option<Permission> = None;
        let mut matched: Option<RuleMatch> = None;
        for (src, rules) in candidates.iter() {
            if let Some(m) = lookup(*src, rules.as_slice()) {
                matched = Some(m.clone());
                match verdict {
                    None => verdict = Some(m.rule.permission),
                    Some(Permission::Allow) if m.rule.permission == Permission::Deny => verdict = Some(Permission::Deny),
                    Some(Permission::Deny) if m.rule.permission == Permission::Allow => { /* Deny wins (defense in depth) */ }
                    _ => {}
                }
                if matches!(verdict, Some(Permission::Deny)) { break; }
            }
        }

        let default_perm = *self.inner.default.lock().unwrap();
        let mut final_perm = verdict.unwrap_or(default_perm);
        if final_perm == Permission::Ask {
            let req = ApprovalRequest {
                agent_id: ctx.agent_id.map(str::to_string),
                tool: ctx.tool.map(str::to_string),
                operation: op.label().to_string(),
                target: scope.label(),
                reason: ctx.reason.map(str::to_string),
                risk: risk_for(op),
            };
            let resp: ApprovalResponse = self.inner.approval.lock().unwrap().request(&req);
            final_perm = resp.permission;
            // Persist session-grant if scope allows.
            if resp.permission == Permission::Allow && matches!(resp.scope, ApprovalScope::Session | ApprovalScope::Project) {
                self.grant_session(Rule::new(op, scope.clone(), Permission::Allow));
            }
        }

        let rec = DecisionRecord {
            timestamp: Utc::now(),
            agent_id: ctx.agent_id.map(str::to_string),
            tool: ctx.tool.map(str::to_string),
            operation: op.label().to_string(),
            target: scope.label(),
            verdict: final_perm,
            matched_rule: matched.as_ref().map(|m| m.summary()),
            reason: ctx.reason.map(str::to_string),
        };
        self.inner.log.record(rec.clone());
        self.inner.sink.lock().unwrap().on_decision(&rec);
        rec
    }

    /// Decide whether a Bash command is allowed (hard/soft/safe classification).
    pub fn decide_bash(&self, agent_id: Option<&str>, command: &str) -> DecisionRecord {
        let (level, _) = classify_bash_with_label(command);
        let perm = match level {
            BashLevel::Hard => Permission::Deny,
            BashLevel::Soft => Permission::Ask,
            BashLevel::Safe => {
                // Fall through to the bash rule.
                self.decide(Operation::Execute, &ResourceScope::CommandSubstring { value: command.to_string() },
                    DecisionContext { agent_id, tool: Some("bash"), role: None, reason: Some(command) })
                .verdict
            }
        };
        DecisionRecord {
            timestamp: Utc::now(),
            agent_id: agent_id.map(str::to_string),
            tool: Some("bash".to_string()),
            operation: "execute".to_string(),
            target: command.to_string(),
            verdict: perm,
            matched_rule: Some(format!("bash-class:{level:?}")),
            reason: None,
        }
    }
}

fn risk_for(op: Operation) -> super::approval::Risk {
    match op {
        Operation::Read | Operation::Create => super::approval::Risk::Low,
        Operation::Write => super::approval::Risk::Medium,
        Operation::Execute => super::approval::Risk::High,
        Operation::Delete | Operation::Network | Operation::Install | Operation::Admin => super::approval::Risk::Critical,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DecisionContext<'a> {
    pub agent_id: Option<&'a str>,
    pub tool: Option<&'a str>,
    pub role: Option<&'a str>,
    pub reason: Option<&'a str>,
}

impl<'a> Default for DecisionContext<'a> {
    fn default() -> Self { Self { agent_id: None, tool: None, role: None, reason: None } }
}

/// Backward-compatible helpers on the v0.11 `Policy`.
impl Policy {
    pub fn from_config(cfg: &aether_config::PermissionsConfig) -> Self {
        Self {
            read: Permission::parse(&cfg.read),
            edit: Permission::parse(&cfg.edit),
            bash: Permission::parse(&cfg.bash),
            delete: Permission::parse(&cfg.delete),
            git_commit: Permission::parse(&cfg.git_commit),
            network: Permission::parse(&cfg.network),
        }
    }

    pub fn value_for(&self, cat: &str) -> Permission {
        match cat {
            "read" => self.read,
            "edit" => self.edit,
            "bash" => self.bash,
            "delete" => self.delete,
            "git_commit" => self.git_commit,
            "network" => self.network,
            _ => Permission::Ask,
        }
    }

    pub fn check_bash(&self, command: &str) -> super::scope::ResourceScope {
        let _ = self.value_for("bash");
        let _ = command;
        ResourceScope::CommandSubstring { value: command.to_string() }
    }

    /// Build the rule list equivalent of this policy.
    pub fn to_rules(&self) -> Vec<Rule> {
        vec![
            Rule::new(Operation::Read, ResourceScope::Any, self.read),
            Rule::new(Operation::Write, ResourceScope::Any, self.edit),
            Rule::new(Operation::Execute, ResourceScope::Any, self.bash),
            Rule::new(Operation::Delete, ResourceScope::Any, self.delete),
            Rule::new(Operation::Admin, ResourceScope::Glob { value: "git/**".into() }, self.git_commit),
            Rule::new(Operation::Network, ResourceScope::Any, self.network),
        ]
    }
}

// ---- Bash classification preserved from v0.11 ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BashLevel { Hard, Soft, Safe }

const HARD_DENY_PATTERNS: &[&str] = &[
    "rm -rf", "sudo ", "git reset --hard", "git push --force", "git push -f",
    "chmod -R", "mkfs", "dd if=", "curl | sh", "shutdown", "reboot", "halt",
    "poweroff", ":(){:|:&};:", "del /f /s", "format.com", "fdisk", "diskpart",
    "> /etc/", "> /boot/", "rm -rf /", "rm -rf ~", "rm -fr",
];
const SOFT_DENY_PATTERNS: &[&str] = &[
    "rm -r", "rm -f", "git push", "git checkout -- ", "git clean -fd",
    "git stash drop", "git branch -D", "git tag -d",
    "kill -9", "killall", "pkill", "chmod 777", "chmod 666",
];

pub fn classify_bash(command: &str) -> BashLevel {
    let lower = command.to_lowercase();
    if HARD_DENY_PATTERNS.iter().any(|p| lower.contains(p)) { return BashLevel::Hard; }
    if SOFT_DENY_PATTERNS.iter().any(|p| lower.contains(p)) { return BashLevel::Soft; }
    BashLevel::Safe
}

pub fn classify_bash_with_label(command: &str) -> (BashLevel, &'static str) {
    let lower = command.to_lowercase();
    if let Some(p) = HARD_DENY_PATTERNS.iter().find(|p| lower.contains(*p)) { return (BashLevel::Hard, p); }
    if let Some(p) = SOFT_DENY_PATTERNS.iter().find(|p| lower.contains(*p)) { return (BashLevel::Soft, p); }
    (BashLevel::Safe, "")
}

pub fn is_dangerous(command: &str) -> bool {
    classify_bash(command) == BashLevel::Hard
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::{AllowAllChannel, DenyAllChannel};

    #[test]
    fn default_is_ask_when_no_rule_matches() {
        // Default = Ask → must consult the approval channel. DenyAllChannel ⇒ Deny.
        let e = PermissionEngine::new().with_default(Permission::Ask)
            .with_approval(Arc::new(DenyAllChannel));
        let rec = e.decide(Operation::Network, &ResourceScope::Host { value: "evil.example".into() },
            DecisionContext::default());
        assert_eq!(rec.verdict, Permission::Deny); // approval denied
    }

    #[test]
    fn global_rule_wins_over_default() {
        let mut e = PermissionEngine::new().with_default(Permission::Ask);
        e.add_global(Rule::new(Operation::Network, ResourceScope::Host { value: "api.openai.com".into() }, Permission::Allow));
        e.set_approval(Arc::new(DenyAllChannel));
        let rec = e.decide(Operation::Network, &ResourceScope::Host { value: "api.openai.com".into() },
            DecisionContext::default());
        assert_eq!(rec.verdict, Permission::Allow);
    }

    #[test]
    fn deny_layer_beats_allow_layer() {
        let e = PermissionEngine::new()
            .with_default(Permission::Ask)
            .with_approval(Arc::new(AllowAllChannel));
        e.add_global(Rule::new(Operation::Write, ResourceScope::Any, Permission::Allow));
        e.add_project(Rule::new(Operation::Write, ResourceScope::Glob { value: "**/.env".into() }, Permission::Deny));
        let rec = e.decide(Operation::Write, &ResourceScope::Path { value: "/repo/.env".into() },
            DecisionContext::default());
        assert_eq!(rec.verdict, Permission::Deny);
    }

    #[test]
    fn bash_classifier_flags_rm_rf() {
        assert_eq!(classify_bash("rm -rf /"), BashLevel::Hard);
        assert_eq!(classify_bash("ls -la"), BashLevel::Safe);
    }

    #[test]
    fn glob_matches_double_star() {
        let r = Rule::new(Operation::Read, ResourceScope::Glob { value: "**/.env".into() }, Permission::Deny);
        assert!(r.matches(Operation::Read, &ResourceScope::Path { value: "/x/y/.env".into() }));
    }
}
