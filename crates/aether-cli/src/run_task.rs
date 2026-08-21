//! `run_task` — the shared task runner used by both the CLI binary and the
//! desktop. The CLI calls it from `main.rs`; the desktop calls it directly
//! in-process via the Tauri command, so no subprocess is ever spawned.
//!
//! Output is delivered through an [`OutputSink`] so the same code drives the
//! terminal stdout and the Tauri `task-output` event stream.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use aether_core::agent_loop::Agent;
use aether_core::mode::Mode;
use aether_models::ModelProvider;
use aether_mind::{skills::SkillIndex, Mind};
use aether_permissions::Permission;
use aether_sessions::SessionStore;
use aether_tools::Tool;

use tokio::sync::Notify;

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub task: Option<String>,
    pub prompt: Option<String>,
    pub plan: bool,
    pub local: bool,
    pub rollback: Option<String>,
    pub debug: bool,
    pub json: bool,
    pub traces: bool,
    pub resume: Option<String>,
    pub worktree: bool,
    pub background: Option<String>,
    pub session_id: Option<String>,
    pub config: Option<PathBuf>,
    pub tui: bool,
    /// v0.17: provider registry for per-session role assignment. When present
    /// alongside `role_assignments`, the gateway is assembled from these
    /// explicit bindings instead of the global `[agent]`/`[models]` config.
    pub providers: Option<Vec<aether_config::ProviderEntry>>,
    /// v0.17: per-session role assignments (executor/controller/reviewer).
    pub role_assignments: Option<aether_config::RoleAssignments>,
    /// v0.17: workspace folder to run the task in (sets the agent cwd).
    pub workspace_path: Option<PathBuf>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            task: None,
            prompt: None,
            plan: false,
            local: false,
            rollback: None,
            debug: false,
            json: false,
            traces: false,
            resume: None,
            worktree: false,
            background: None,
            session_id: None,
            config: None,
            tui: false,
            providers: None,
            role_assignments: None,
            workspace_path: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TaskEvent {
    /// A line of agent output. `stream` is "stdout" or "stderr" to mirror the
    /// Tauri command's wire format.
    Line { stream: &'static str, line: String },
    /// The agent loop ended. Either cleanly or with an error.
    Exit { code: i32, success: bool },
    /// The agent loop returned an error result. The sink gets a final Exit
    /// event with `success=false` alongside this.
    Error { message: String },
    /// Authoritative task-state update from the 3-LLM state machine.
    TaskState { json: String },
}

/// Output sink for the agent loop. Implementations decide where the lines go
/// (terminal stdout, Tauri event bus, an in-memory buffer, …).
pub type OutputSink = Arc<dyn Fn(TaskEvent) + Send + Sync>;

/// Run the agent loop. Returns when the agent finishes (cleanly or with an
/// error) or when `cancel` is notified.
///
/// `sink` is invoked for every line of agent output. The caller is responsible
/// for surfacing those lines to the user (terminal, Tauri events, …).
pub async fn run(
    opts: RunOptions,
    cancel: Arc<Notify>,
    sink: OutputSink,
) -> anyhow::Result<()> {
    let cfg_path = opts.config.clone().unwrap_or_else(aether_config::Config::default_path);

    // Background mode is a developer escape hatch — it intentionally spawns
    // a child process so the parent can return immediately. The desktop does
    // not use this path; the CLI does, and the child is fully detached with
    // no visible console window.
    if let Some(task) = &opts.background {
        return run_background(opts.clone(), task, &cfg_path, &cancel, &sink).await;
    }

    let mut cfg = match aether_config::Config::load(opts.config.clone()) {
        Ok(c) => c,
        Err(e) => {
            (sink)(TaskEvent::Error { message: format!("AETHER: could not read config at {}: {}", cfg_path.display(), e) });
            (sink)(TaskEvent::Exit { code: 1, success: false });
            return Ok(());
        }
    };

    // Local mode: point every model at the local OpenAI-compatible endpoint.
    if opts.local {
        for m in cfg.models.values_mut() {
            m.base_url = cfg.agent.local_endpoint.clone();
        }
        emit(&sink, "stdout", &format!("LOCAL MODE — models → {}", cfg.agent.local_endpoint));
    }

    // Rollback: restore the last file-write checkpoint for a session.
    if let Some(session) = &opts.rollback {
        return run_rollback(&cfg, session, &sink).await;
    }

    // v0.17: per-session role assignments take precedence over the global
    // [agent]/[models] config. When a provider registry + role assignments
    // are supplied, the gateway is assembled from those explicit bindings.
    let use_session_bindings = opts.providers.is_some() && opts.role_assignments.is_some();

    if !use_session_bindings
        && (cfg.model(&cfg.agent.controller_model).is_none() || cfg.model(&cfg.agent.executor_model).is_none())
    {
        emit(&sink, "stderr", "AETHER: no controller or executor model configured. Open the desktop Settings and configure at least Model 1.");
        (sink)(TaskEvent::Exit { code: 2, success: false });
        return Ok(());
    }

    let mut policy = aether_permissions::Policy::from_config(&cfg.permissions);
    if opts.plan {
        policy.edit = Permission::Deny;
        policy.delete = Permission::Deny;
        policy.git_commit = Permission::Deny;
        policy.bash = Permission::Ask;
        emit(&sink, "stderr", "PLANNING MODE — read-only, no file changes will be made.");
    }

    // ---- v0.15 Model Gateway ----
    // v0.17: assemble from per-session role assignments when provided,
    // otherwise fall back to the global [agent]/[models] config.
    let gateway_bundle = if use_session_bindings {
        let providers_reg = opts.providers.clone().unwrap_or_default();
        let assignments = opts.role_assignments.clone().unwrap_or_default();
        match aether_gateway::GatewayBundle::from_providers(
            &providers_reg,
            &assignments,
            aether_gateway::GatewayConfig::default(),
        ) {
            Ok(b) => b,
            Err(e) => {
                emit(&sink, "stderr", &format!("gateway assembly failed: {e}"));
                (sink)(TaskEvent::Exit { code: 2, success: false });
                return Ok(());
            }
        }
    } else {
        match aether_gateway::GatewayBundle::from_config(&cfg, aether_gateway::GatewayConfig::default()) {
            Ok(b) => b,
            Err(e) => {
                emit(&sink, "stderr", &format!("gateway assembly failed: {e}"));
                (sink)(TaskEvent::Exit { code: 2, success: false });
                return Ok(());
            }
        }
    };
    let providers = gateway_bundle.providers.clone();
    let controller: Arc<dyn ModelProvider> = gateway_bundle.controller.clone();
    emit(&sink, "stdout", "model gateway: explicit per-role bindings (no routing, no fallback)");

    let reviewer: Option<Arc<dyn ModelProvider>> = gateway_bundle
        .gateway
        .provider_for(aether_gateway::Role::Reviewer)
        .ok()
        .map(|rp| rp.provider);

    // Memory engine.
    let (mind, embedder): (Option<Arc<Mind>>, Option<Arc<dyn ModelProvider>>) = if cfg.memory.enabled {
        let mind_path = aether_config::expand_tilde(&cfg.memory.path);
        match Mind::open(&mind_path) {
            Ok(m) => (Some(m), Some(controller.clone())),
            Err(_) => (None, None),
        }
    } else {
        (None, None)
    };
    let skills = match SkillIndex::discover_with_bundled(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))) {
        s => s,
    };

    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    for tool in aether_tools::default_tools() {
        tools.insert(tool.name().to_string(), tool);
    }
    for t in aether_tools::analysis::analysis_tools() {
        tools.insert(t.name().to_string(), t);
    }
    if let Some(m) = &mind {
        for t in aether_mind::tools::memory_tools(m.clone(), embedder.clone()) {
            tools.insert(t.name().to_string(), t);
        }
    }
    for t in aether_mind::tools::skill_tools(skills.clone()) {
        tools.insert(t.name().to_string(), t);
    }

    // MCP client (best-effort).
    for srv in &cfg.mcp.servers {
        match aether_tools::mcp::McpClient::connect(&srv.command, &srv.args).await {
            Ok(client) => match client.list_tools().await {
                Ok(infos) => {
                    for info in infos {
                        let t: Arc<dyn Tool> =
                            Arc::new(aether_tools::mcp::McpTool::from_info(client.clone(), info));
                        tools.insert(t.name().to_string(), t);
                    }
                    emit(&sink, "stdout", &format!("connected to MCP server '{}'", srv.name));
                }
                Err(e) => emit(&sink, "stderr", &format!("mcp '{}' list failed: {e}", srv.name)),
            },
            Err(e) => emit(&sink, "stderr", &format!("mcp '{}' connect failed: {e}", srv.name)),
        }
    }
    let subagent_tools = tools;

    let store = match SessionStore::open(&aether_config::Config::default_dir().join("sessions.db")) {
        Ok(s) => s,
        Err(e) => {
            emit(&sink, "stderr", &format!("session store open failed: {e}"));
            (sink)(TaskEvent::Exit { code: 1, success: false });
            return Ok(());
        }
    };

    let (session_id, resume_plan): (String, Option<String>) = if let Some(id) = &opts.resume {
        let plan = store.get(id).ok().flatten().and_then(|m| m.plan);
        emit(&sink, "stdout", &format!("resuming session {id}"));
        (id.clone(), plan)
    } else if let Some(id) = &opts.session_id {
        (id.clone(), None)
    } else {
        match store.new_session() {
            Ok(s) => (s, None),
            Err(e) => {
                emit(&sink, "stderr", &format!("session create failed: {e}"));
                (sink)(TaskEvent::Exit { code: 1, success: false });
                return Ok(());
            }
        }
    };

    // v0.17: when a workspace folder is supplied, run the agent inside it.
    let base_cwd = opts
        .workspace_path
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let run_cwd = if opts.worktree {
        match make_worktree(&base_cwd) {
            Ok(p) => {
                emit(&sink, "stdout", &format!("working in git worktree: {}", p.display()));
                p
            }
            Err(e) => {
                emit(&sink, "stderr", &format!("worktree setup failed: {e}"));
                (sink)(TaskEvent::Exit { code: 1, success: false });
                return Ok(());
            }
        }
    } else {
        base_cwd
    };

    emit(&sink, "stdout", "AETHER");
    emit(&sink, "stdout", &format!("session: {session_id}"));

    // ---- v0.12 subsystems ----
    let permission_engine = {
        let pe = aether_permissions::PermissionEngine::from_policy(&policy);
        pe.add_global(
            aether_permissions::Rule::new(
                aether_permissions::Operation::Read,
                aether_permissions::ResourceScope::Glob { value: "**/.env".into() },
                aether_permissions::Permission::Deny,
            )
            .with_reason("secrets are not readable by default"),
        );
        Arc::new(pe)
    };
    let context_manager = Arc::new(aether_context::ContextManager::new(
        aether_context::ContextManagerConfig::new(
            "main",
            cfg.agent.executor_model.clone(),
            cfg.context.max_tokens,
        ),
    ));
    let snapshots_root = aether_config::expand_tilde(&format!("~/.aether/snapshots/{}", session_id));
    let snapshots = Arc::new(std::sync::Mutex::new(
        aether_sessions::SnapshotManager::open(snapshots_root).unwrap_or_else(|_| {
            let tmp = std::env::temp_dir().join(format!("aether-snap-{session_id}"));
            aether_sessions::SnapshotManager::open(tmp).expect("snapshots dir")
        }),
    ));

    // ---- v0.13 subsystems ----
    let plugins = Arc::new(aether_plugin::Registry::new());
    let evidence = Arc::new(aether_evidence::EvidenceBag::new());
    let context_workspace = Arc::new(aether_context::ContextWorkspace::new());

    // ---- Session compaction (structured checkpoints) ----
    // The compactor uses the session's configured Model 2 (controller) to
    // generate checkpoints. No routing, no model switching.
    let compactor = Arc::new(aether_context::SessionCompactor::new(
        controller.clone(),
        cfg.agent.controller_model.clone(),
        cfg.context.max_tokens,
        Arc::new(aether_core::compaction_store::SessionCheckpointStore::new(
            aether_config::Config::default_dir().join("sessions.db"),
        )),
    ));

    let agent = Agent::new(
        controller,
        cfg.agent.controller_model.clone(),
        cfg.agent.executor_model.clone(),
        providers,
        Some(store.clone()),
        session_id.clone(),
        mind,
        embedder,
        cfg.memory.auto_extract,
        cfg.memory.memory_top_k,
        run_cwd,
        policy,
        subagent_tools,
        cfg.subagents.enabled,
        cfg.agent.max_iterations,
        cfg.context.max_tokens,
        cfg.agent.loop_budget,
        reviewer,
        cfg.agent.reviewer_model.clone(),
        cfg.frontend.clone(),
    )
    .with_permission_engine(permission_engine)
    .with_context_manager(context_manager)
    .with_snapshots(snapshots)
    .with_plugins(plugins)
    .with_evidence(evidence)
    .with_context_workspace(context_workspace)
    .with_compactor(compactor)
    .with_task_event_sink({
        let sink2 = sink.clone();
        Arc::new(move |ev: aether_core::task_state::TaskEventKind| {
            if let Ok(json) = serde_json::to_string(&ev) {
                (sink2)(TaskEvent::TaskState { json });
            }
        })
    })
    .with_cancel(cancel);

    let format = |task: &str| -> String {
        if opts.plan {
            format!("{task}\n\n(PLAN MODE — read-only. Produce the structured PLAN document; do not modify files.)")
        } else {
            task.to_string()
        }
    };

    let current_mode: Mode = if opts.plan { Mode::Plan } else { Mode::Build };

    let task = match opts.task.clone().or(opts.prompt.clone()) {
        Some(t) => t,
        None => {
            emit(&sink, "stderr", "no task provided to run_task");
            (sink)(TaskEvent::Exit { code: 2, success: false });
            return Ok(());
        }
    };

    let outcome = agent
        .run(&format(&task), current_mode, resume_plan.as_deref(), opts.resume.as_deref())
        .await;

    match outcome {
        Ok(o) => {
            if opts.json {
                (sink)(TaskEvent::Line {
                    stream: "stdout",
                    line: serde_json::json!({
                        "mode": current_mode.label(),
                        "plan": o.plan,
                        "result": o.result,
                        "review": o.review,
                        "test": o.test,
                        "engineering": o.engineering
                    })
                    .to_string(),
                });
            } else {
                if !o.plan.is_empty() {
                    (sink)(TaskEvent::Line { stream: "stdout", line: format!("[PLAN]\n{}", o.plan) });
                }
                if !o.result.is_empty() {
                    (sink)(TaskEvent::Line { stream: "stdout", line: o.result.clone() });
                }
            }
            (sink)(TaskEvent::Exit { code: 0, success: true });
        }
        Err(e) => {
            (sink)(TaskEvent::Error { message: e.to_string() });
            (sink)(TaskEvent::Exit { code: 1, success: false });
        }
    }
    Ok(())
}

fn emit(sink: &OutputSink, stream: &'static str, line: &str) {
    (sink)(TaskEvent::Line { stream, line: line.to_string() });
}

async fn run_rollback(
    _cfg: &aether_config::Config,
    session: &str,
    sink: &OutputSink,
) -> anyhow::Result<()> {
    let store = SessionStore::open(&aether_config::Config::default_dir().join("sessions.db"))?;
    match store.last_checkpoint(session)? {
        Some(cp) => {
            let full = std::env::current_dir()?.join(&cp.path);
            match cp.before_content {
                Some(content) => {
                    if let Some(parent) = full.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&full, content)?;
                    emit(sink, "stdout", &format!("[rollback] restored {} to pre-{} state", cp.path, cp.tool));
                }
                None => {
                    if full.exists() {
                        std::fs::remove_file(&full)?;
                        emit(sink, "stdout", &format!("[rollback] removed {} (did not exist before)", cp.path));
                    } else {
                        emit(sink, "stdout", "[rollback] nothing to restore");
                    }
                }
            }
        }
        None => emit(sink, "stderr", "[rollback] no checkpoint found for session"),
    }
    (sink)(TaskEvent::Exit { code: 0, success: true });
    Ok(())
}

async fn run_background(
    opts: RunOptions,
    task: &str,
    cfg_path: &Path,
    _cancel: &Arc<Notify>,
    sink: &OutputSink,
) -> anyhow::Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let log_dir = aether_config::Config::default_dir().join("background");
    std::fs::create_dir_all(&log_dir)?;
    let log = log_dir.join(format!("{id}.log"));
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg(task).arg("--session-id").arg(&id);
    if let Some(c) = &opts.config {
        cmd.arg("--config").arg(c);
    }
    if opts.local {
        cmd.arg("--local");
    }
    if opts.plan {
        cmd.arg("--plan");
    }
    if opts.json {
        cmd.arg("--json");
    }
    if opts.debug {
        cmd.arg("--debug");
    }
    if opts.traces {
        cmd.arg("--traces");
    }
    if let Some(r) = &opts.resume {
        cmd.arg("--resume").arg(r);
    }
    cmd.stdout(std::fs::File::create(&log)?)
        .stderr(std::fs::File::create(&log)?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NO_WINDOW so no console flashes.
        cmd.creation_flags(0x00000008 | 0x08000000);
    }
    let _ = cmd.spawn()?;
    emit(sink, "stdout", &format!("[background] session {id}"));
    emit(sink, "stdout", &format!("log: {}", log.display()));
    emit(sink, "stdout", &format!("inspect later:  aether --resume {id}   |   aether --traces (after --resume)"));
    (sink)(TaskEvent::Exit { code: 0, success: true });
    Ok(())
}

fn make_worktree(cwd: &Path) -> anyhow::Result<PathBuf> {
    // Reuse the CLI's worktree setup.
    let id = uuid::Uuid::new_v4().simple().to_string();
    let branch = format!("aether-{id}");
    let path = cwd.parent().unwrap_or(cwd).join(format!(
        "{}-{}",
        cwd.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "aether".into()),
        &id[..8]
    ));
    let status = std::process::Command::new("git")
        .args(["worktree", "add", "-b", &branch])
        .arg(&path)
        .current_dir(cwd)
        .status()?;
    if !status.success() {
        anyhow::bail!("git worktree add failed: {status}");
    }
    Ok(path)
}
