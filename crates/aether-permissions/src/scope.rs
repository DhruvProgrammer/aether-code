//! Operation kinds + resource scopes.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Read,
    Write,
    Create,
    Delete,
    Execute,
    Network,
    Install,
    Admin,
}

impl Operation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Create => "create",
            Self::Delete => "delete",
            Self::Execute => "execute",
            Self::Network => "network",
            Self::Install => "install",
            Self::Admin => "admin",
        }
    }
}

/// Identifies what is being targeted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceScope {
    /// A file or directory path; the matcher handles both.
    Path { value: String },
    /// A glob pattern (e.g. `*.env`, `**/secrets/*`).
    Glob { value: String },
    /// A network host:port or scheme://host.
    Host { value: String },
    /// A shell command substring (substring match, like the v0.11 denylist).
    CommandSubstring { value: String },
    /// A tool name.
    Tool { value: String },
    /// A provider id.
    Provider { value: String },
    /// A model id.
    Model { value: String },
    /// An MCP server name.
    Mcp { value: String },
    /// An env var name.
    EnvVar { value: String },
    /// A secret identifier.
    Secret { value: String },
    /// Wildcard (matches anything).
    Any,
}

impl ResourceScope {
    pub fn matches(&self, op: &ResourceScope) -> bool {
        match (self, op) {
            (Self::Any, _) => true,
            (Self::Path { value: a }, Self::Path { value: b }) => path_glob_match(a, b),
            (Self::Glob { value: a }, Self::Path { value: b }) => path_glob_match(a, b),
            (Self::Glob { value: a }, Self::Glob { value: b }) => path_glob_match(a, b),
            (Self::Host { value: a }, Self::Host { value: b }) => a == b,
            (Self::CommandSubstring { value: a }, Self::CommandSubstring { value: b }) => b.contains(a.as_str()),
            (Self::Tool { value: a }, Self::Tool { value: b }) => a == b,
            (Self::Provider { value: a }, Self::Provider { value: b }) => a == b,
            (Self::Model { value: a }, Self::Model { value: b }) => a == b,
            (Self::Mcp { value: a }, Self::Mcp { value: b }) => a == b,
            (Self::EnvVar { value: a }, Self::EnvVar { value: b }) => a == b,
            (Self::Secret { value: a }, Self::Secret { value: b }) => a == b,
            _ => false,
        }
    }
}

fn path_glob_match(pattern: &str, value: &str) -> bool {
    crate::glob::glob_match(pattern, value)
}
