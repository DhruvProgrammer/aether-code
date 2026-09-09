//! Built-in tool plugins — the Provider role of the tool seam.
//!
//! Each plugin wraps a group of [`Tool`] implementations and
//! contributes them to the host [`ToolService`](aether_runtime::ToolService)
//! as [`contributors`](aether_runtime::ToolContributor) when its
//! composition row activates. Disposal unregisters exactly the tools
//! the row contributed.
//!
//! Groups mirror construction boundaries (not role allowlists — those
//! stay in `registry::canonical_toolsets` + `Executor.allowed_tools`):
//!
//! | Row id | Factory | Tools |
//! |---|---|---|
//! | `tools-fs` | `builtin:tools-fs` | read/write/list/grep |
//! | `tools-git` | `builtin:tools-git` | git status/diff/log/… |
//! | `tools-terminal` | `builtin:tools-terminal` | execute_command |
//! | `tools-analysis` | `builtin:tools-analysis` | analyze/status |
//! | `tools-memory` | `builtin:tools-memory` | memory_* (prebuilt) |
//! | `tools-skills` | `builtin:tools-skills` | skill_* (prebuilt) |
//! | `tools-mcp` | `builtin:tools-mcp` | MCP-bridged (prebuilt) |
//!
//! [`base_profile`] returns the bundled base rows (exactly today's
//! tool set — first boot is behavior-identical).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use aether_runtime::{
    BoxFuture, Plugin, PluginContext, PluginInfo, Row, ToolContributor, ToolFault, ToolFaultKind,
};
use async_trait::async_trait;
use serde_json::Value;

use crate::{ExecuteCommandTool, GrepTool, ListDirectoryTool, ReadFileTool, Tool, WriteFileTool};

/// Factory names understood by the base profile.
pub const FACTORY_FS: &str = "builtin:tools-fs";
pub const FACTORY_GIT: &str = "builtin:tools-git";
pub const FACTORY_TERMINAL: &str = "builtin:tools-terminal";
pub const FACTORY_ANALYSIS: &str = "builtin:tools-analysis";
pub const FACTORY_MEMORY: &str = "builtin:tools-memory";
pub const FACTORY_SKILLS: &str = "builtin:tools-skills";
pub const FACTORY_MCP: &str = "builtin:tools-mcp";

/// Adapt an `Arc<dyn Tool>` to a seam contributor owned by `owner`.
///
/// Output vocabulary: `{output, is_error}`. Failures map to
/// [`ToolFaultKind::Failed`] with the message preserved — the loop
/// matches on `kind`, never on message text.
pub fn adapt(tool: Arc<dyn Tool>, owner: &str) -> ToolContributor {
    let name = tool.name().to_string();
    let owner = owner.to_string();
    let description = tool.description().to_string();
    let parameters = tool.json_schema();
    let execute = Arc::new(
        move |args: Value, cwd: PathBuf| -> BoxFuture<'static, Result<Value, ToolFault>> {
            let tool = tool.clone();
            Box::pin(async move {
                let ctx = crate::ToolContext { cwd };
                match tool.execute(args, &ctx).await {
                    Ok(res) => Ok(serde_json::json!({
                        "output": res.output,
                        "is_error": res.is_error,
                    })),
                    Err(e) => Err(ToolFault::new(ToolFaultKind::Failed, e.to_string())),
                }
            })
        },
    );
    ToolContributor {
        name,
        owner: owner.clone(),
        scope: aether_runtime::ScopeKey::GLOBAL,
        description,
        parameters,
        execute,
    }
}

/// A plugin contributing a fixed group of tools.
///
/// Construction stays exactly where it is today (each `*_tools()`
/// constructor); the plugin only owns *registration*. `registered()`
/// exposes the built tools so `run_task` can keep feeding the legacy
/// executor tool map during the P0 transition (P1 resolves the
/// executor exclusively through the seam).
pub struct ToolPlugin {
    info: PluginInfo,
    /// Config keys this plugin understands (everything else warns).
    known_keys: Vec<String>,
    tools: Vec<Arc<dyn Tool>>,
    registered: Mutex<Vec<Arc<dyn Tool>>>,
}

impl std::fmt::Debug for ToolPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolPlugin")
            .field("info", &self.info)
            .field("tools", &self.tools.len())
            .finish_non_exhaustive()
    }
}

impl ToolPlugin {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Self {
        let id = id.into();
        Self {
            info: PluginInfo::new(id, name, env!("CARGO_PKG_VERSION"), description),
            known_keys: Vec::new(),
            tools,
            registered: Mutex::new(Vec::new()),
        }
    }

    /// Stable row id (also the contribution owner tag).
    pub fn id(&self) -> &str {
        &self.info.id
    }

    /// Tools built by the last `apply` (empty before boot).
    pub fn registered(&self) -> Vec<Arc<dyn Tool>> {
        self.registered.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn fs() -> Self {
        Self::new(
            "tools-fs",
            "Filesystem tools",
            "read/write/list/grep file tools",
            vec![
                Arc::new(ReadFileTool),
                Arc::new(WriteFileTool),
                Arc::new(ListDirectoryTool),
                Arc::new(GrepTool),
            ],
        )
    }

    pub fn git() -> Self {
        Self::new("tools-git", "Git tools", "git status/diff/log/branch tools", crate::git::git_tools())
    }

    pub fn terminal() -> Self {
        Self::new(
            "tools-terminal",
            "Terminal tool",
            "shell command execution (policy-routed)",
            vec![Arc::new(ExecuteCommandTool)],
        )
    }

    pub fn analysis() -> Self {
        Self::new(
            "tools-analysis",
            "Code analysis tools",
            "sonarqube-backed analyze/status tools",
            crate::analysis::analysis_tools(),
        )
    }

    /// Prebuilt groups (memory/skills/MCP) enter through here — the
    /// plugin owns registration, the existing constructors own
    /// construction.
    pub fn prebuilt(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Self {
        Self::new(id, name, description, tools)
    }
}

#[async_trait]
impl Plugin for ToolPlugin {
    fn info(&self) -> PluginInfo {
        self.info.clone()
    }

    async fn apply(&self, ctx: &mut PluginContext) -> anyhow::Result<()> {
        ctx.warn_unknown_keys(&self.known_keys.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        *self.registered.lock().unwrap_or_else(|e| e.into_inner()) = self.tools.clone();
        for tool in &self.tools {
            let effect = ctx.host.tools.register(adapt(tool.clone(), &self.info.id));
            ctx.effects.push(effect);
        }
        ctx.host.tools.emit_change(&ctx.host.bus, &self.info.id, "registered").await;
        Ok(())
    }
}

/// The bundled base profile: exactly today's tool set, as rows.
/// Layer order puts user patches and overlays after these; same id
/// replaces the whole row.
pub fn base_profile() -> Vec<Row> {
    for_id(&[
        ("tools-fs", FACTORY_FS),
        ("tools-git", FACTORY_GIT),
        ("tools-terminal", FACTORY_TERMINAL),
        ("tools-analysis", FACTORY_ANALYSIS),
        ("tools-memory", FACTORY_MEMORY),
        ("tools-skills", FACTORY_SKILLS),
        ("tools-mcp", FACTORY_MCP),
    ])
}

fn for_id(rows: &[(&str, &str)]) -> Vec<Row> {
    rows.iter().map(|(id, plugin)| Row::new(*id, *plugin)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_runtime::{Composition, PluginHost, ScopeChain, ScopeKey, compose};

    fn test_host() -> (Arc<PluginHost>, Arc<ToolPlugin>) {
        let host = Arc::new(PluginHost::new());
        let plugin = Arc::new(ToolPlugin::fs());
        let p = plugin.clone();
        host.register_factory(FACTORY_FS, Box::new(move || p.clone()));
        (host, plugin)
    }

    #[tokio::test]
    async fn fs_plugin_registers_and_disposes() {
        let (host, plugin) = test_host();
        let c = Composition {
            rows: vec![aether_runtime::ComposedRow {
                row: Row::new("tools-fs", FACTORY_FS),
                origin: aether_runtime::Origin::Base,
                replaced: false,
            }],
            warnings: vec![],
        };
        let report = host.boot(c).await.unwrap();
        assert_eq!(report.activated, vec!["tools-fs"]);
        assert_eq!(plugin.registered().len(), 4);
        let chain = ScopeChain::for_scope(ScopeKey::GLOBAL);
        let read = host.inner().tools.resolve("read_file", &chain).unwrap();
        assert_eq!(read.owner, "tools-fs");
        // Schema assembly inputs survive the adaptation.
        assert!(read.description.len() > 10);
        assert!(read.parameters.is_object());
        // Body executes through the seam contributor.
        let out = (read.execute)(serde_json::json!({"path": "Cargo.toml"}), PathBuf::from("C:\\Users\\MR.PC\\Desktop\\bcode\\aether\\crates\\aether-tools"))
            .await
            .unwrap();
        assert_eq!(out["is_error"], serde_json::json!(false));
        host.shutdown().await;
        assert!(host.inner().tools.resolve("read_file", &chain).is_none());
    }

    #[tokio::test]
    async fn full_base_profile_boots_today_toolset() {
        // Every base row has a factory here except the prebuilt trio
        // (memory/skills/mcp come from run_task with live handles).
        let host = Arc::new(PluginHost::new());
        host.register_factory(FACTORY_FS, Box::new(|| Arc::new(ToolPlugin::fs())));
        host.register_factory(FACTORY_GIT, Box::new(|| Arc::new(ToolPlugin::git())));
        host.register_factory(FACTORY_TERMINAL, Box::new(|| Arc::new(ToolPlugin::terminal())));
        host.register_factory(FACTORY_ANALYSIS, Box::new(|| Arc::new(ToolPlugin::analysis())));
        let rows: Vec<Row> = base_profile()
            .into_iter()
            .filter(|r| {
                !["tools-memory", "tools-skills", "tools-mcp"].contains(&r.id.as_str())
            })
            .collect();
        let report = host.boot(compose(rows, vec![])).await.unwrap();
        assert_eq!(report.activated.len(), 4);
        let chain = ScopeChain::for_scope(ScopeKey::GLOBAL);
        let names = host.inner().tools.visible_names(&chain);
        // Matches default_tools() + analysis_tools() surface.
        for expected in [
            "read_file",
            "write_file",
            "list_directory",
            "grep",
            "execute_command",
            "git_status",
            "analyze_code",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}: {names:?}");
        }
        host.shutdown().await;
    }
}
