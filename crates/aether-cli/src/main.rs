//! CLI entrypoint (spec §23, §24). OpenAI-compatible coding agent.
mod ui;
mod tui;

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use clap::Parser;

use aether_core::agent_loop::Agent;
use aether_core::mode::Mode;
use aether_models::ModelProvider;
use aether_mind::{Mind, skills::SkillIndex};
use aether_permissions::Permission;
use aether_sessions::SessionStore;
use aether_tools::Tool;

#[derive(Parser)]
#[command(name = "aether", version, about = "OpenAI-compatible coding agent")]
pub struct Cli {
    /// Task to run non-interactively.
    task: Option<String>,
    /// Task to run non-interactively (explicit flag).
    #[arg(long)]
    prompt: Option<String>,
    /// Planning mode: read-only, never modifies files; returns a plan.
    #[arg(long)]
    plan: bool,
    /// Local mode: point all models at the local OpenAI-compatible endpoint (spec §6).
    #[arg(long)]
    local: bool,
    /// Roll back the last file-write checkpoint for a session id.
    #[arg(long)]
    rollback: Option<String>,
    /// Verbose tracing.
    #[arg(long)]
    debug: bool,
    /// Emit machine-parseable JSON (exit non-zero on failure).
    #[arg(long)]
    json: bool,
    /// Print the session's trace log (agent actions / decisions / verification) after running.
    #[arg(long)]
    traces: bool,
    /// Resume a previous session id (reloads its engineering state + plan and continues).
    #[arg(long)]
    resume: Option<String>,
    /// Run the agent inside a git worktree so its edits are isolated and reviewable.
    #[arg(long)]
    worktree: bool,
    /// Run a task as a detached background process; prints a session id you can later
    /// inspect with `--resume <id>` / `--traces`.
    #[arg(long)]
    background: Option<String>,
    /// Internal: force a specific session id (used by `--background` to spawn a child).
    #[arg(long, hide = true)]
    session_id: Option<String>,
    /// Path to config.toml (defaults to ~/.aether/config.toml).
    #[arg(long)]
    config: Option<PathBuf>,
    /// Launch the interactive TUI. The TUI is also auto-launched when no task is
    /// supplied and stdin/stdout are both TTYs.
    #[arg(long)]
    tui: bool,
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("AETHER error: {e}");
        pause_if_terminal();
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if cli.debug {
        tracing_subscriber::fmt().with_env_filter("debug").init();
    } else {
        tracing_subscriber::fmt().with_env_filter("info").init();
    }

    // TUI dispatch: when the user runs `aether` with no task and on a real
    // terminal, drop into the ratatui front-end. `--tui` is explicit.
    let has_task = cli.task.is_some() || cli.prompt.is_some();
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if cli.tui || (interactive && !has_task && cli.background.is_none() && cli.resume.is_none() && cli.rollback.is_none() && cli.traces == false) {
        let args: Vec<String> = std::env::args().collect();
        return tui::run_tui(cli, args).await;
    }

    let cfg_path = cli.config.clone().unwrap_or_else(aether_config::Config::default_path);
    let cfg_missing = !cfg_path.exists();
    let mut cfg = match aether_config::Config::load(cli.config.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("AETHER: could not read config at {}: {}", cfg_path.display(), e);
            pause_if_terminal();
            std::process::exit(1);
        }
    };

    // Background run (Phase 4): spawn a detached child process with a fixed session id so the
    // parent can return immediately. The child runs the same binary to completion on its own.
    if let Some(task) = &cli.background {
        let id = uuid::Uuid::new_v4().to_string();
        let log_dir = aether_config::Config::default_dir().join("background");
        std::fs::create_dir_all(&log_dir)?;
        let log = log_dir.join(format!("{id}.log"));
        let exe = std::env::current_exe()?;
        let mut cmd = Command::new(exe);
        cmd.arg(task).arg("--session-id").arg(&id);
        if let Some(c) = &cli.config {
            cmd.arg("--config").arg(c);
        }
        if cli.local {
            cmd.arg("--local");
        }
        if cli.plan {
            cmd.arg("--plan");
        }
        if cli.json {
            cmd.arg("--json");
        }
        if cli.debug {
            cmd.arg("--debug");
        }
        if cli.traces {
            cmd.arg("--traces");
        }
        if let Some(r) = &cli.resume {
            cmd.arg("--resume").arg(r);
        }
        cmd.stdout(std::fs::File::create(&log)?)
            .stderr(std::fs::File::create(&log)?);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x00000200 | 0x00000008); // NEW_PROCESS_GROUP | DETACHED_PROCESS
        }
        let _ = cmd.spawn()?;
        ui::section("[background]", &format!("session {id}"));
        ui::note(&format!("log: {}", log.display()));
        ui::note(&format!(
            "inspect later:  aether --resume {id}   |   aether --traces (after --resume)"
        ));
        return Ok(());
    }

    // Local mode (spec §6): point every model at the local OpenAI-compatible endpoint.
    if cli.local {
        for m in cfg.models.values_mut() {
            m.base_url = cfg.agent.local_endpoint.clone();
        }
        ui::note(&format!("LOCAL MODE — models → {}", cfg.agent.local_endpoint));
    }

    // Rollback mode (spec §15): restore the most recent checkpoint for a session.
    if let Some(session) = &cli.rollback {
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
                        ui::section("[rollback]", &format!("restored {} to pre-{} state", cp.path, cp.tool));
                    }
                    None => {
                        if full.exists() {
                            std::fs::remove_file(&full)?;
                            ui::section("[rollback]", &format!("removed {} (did not exist before)", cp.path));
                        } else {
                            ui::note("nothing to restore");
                        }
                    }
                }
                return Ok(());
            }
            None => {
                ui::error("no checkpoint found for session");
                return Ok(());
            }
        }
    }

    if cfg.model(&cfg.agent.controller_model).is_none() || cfg.model(&cfg.agent.executor_model).is_none() {
        setup_help(&cfg_path, cfg_missing);
        std::process::exit(2);
    }
    let controller_cfg = cfg.model(&cfg.agent.controller_model).unwrap();

    let mut policy = aether_permissions::Policy::from_config(&cfg.permissions);
    if cli.plan {
        // Read-only planning (spec §13): no writes, no destructive git.
        policy.edit = Permission::Deny;
        policy.delete = Permission::Deny;
        policy.git_commit = Permission::Deny;
        policy.bash = Permission::Ask;
        ui::warn("PLANNING MODE — read-only, no file changes will be made.");
    }

    // Build a provider per configured model (spec §8 routing).
    let mut providers: HashMap<String, Arc<dyn ModelProvider>> = HashMap::new();
    for (key, mcfg) in &cfg.models {
        if let Ok(p) = aether_models::build_provider(mcfg) {
            providers.insert(key.clone(), Arc::from(p));
        }
    }
    let controller: Arc<dyn ModelProvider> = Arc::from(aether_models::build_provider(controller_cfg)?);

    // LLM 3 — VISUAL FRONTEND REVIEWER (optional, multimodal). Degrades gracefully when unset.
    let reviewer: Option<Arc<dyn ModelProvider>> = match &cfg.agent.reviewer_model {
        Some(key) => match cfg.model(key) {
            Some(mc) => match aether_models::build_provider(mc) {
                Ok(p) => Some(Arc::from(p)),
                Err(e) => {
                    ui::warn(&format!("reviewer model '{key}' unavailable: {e}"));
                    None
                }
            },
            None => {
                ui::warn(&format!("reviewer model '{key}' not found in [models]; visual review disabled"));
                None
            }
        },
        None => None,
    };

    // Memory engine (spec §9). Embeddings reuse the controller provider.
    let (mind, embedder): (Option<Arc<Mind>>, Option<Arc<dyn ModelProvider>>) = if cfg.memory.enabled {
        let mind_path = aether_config::expand_tilde(&cfg.memory.path);
        let m = Mind::open(&mind_path)?;
        (Some(m), Some(controller.clone()))
    } else {
        (None, None)
    };
    let skills = SkillIndex::discover(&std::env::current_dir()?);

    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    for tool in aether_tools::default_tools() {
        tools.insert(tool.name().to_string(), tool);
    }
    if let Some(m) = &mind {
        for t in aether_mind::tools::memory_tools(m.clone(), embedder.clone()) {
            tools.insert(t.name().to_string(), t);
        }
    }
    for t in aether_mind::tools::skill_tools(skills.clone()) {
        tools.insert(t.name().to_string(), t);
    }

    // MCP client (spec §6): connect to configured external MCP servers and adapt their
    // tools into the agent's toolset. Failures are non-fatal.
    for srv in &cfg.mcp.servers {
        match aether_tools::mcp::McpClient::connect(&srv.command, &srv.args).await {
            Ok(client) => match client.list_tools().await {
                Ok(infos) => {
                    for info in infos {
                        let t: Arc<dyn Tool> =
                            Arc::new(aether_tools::mcp::McpTool::from_info(client.clone(), info));
                        tools.insert(t.name().to_string(), t);
                    }
                    ui::note(&format!("connected to MCP server '{}'", srv.name));
                }
                Err(e) => ui::warn(&format!("mcp '{}' list failed: {e}", srv.name)),
            },
            Err(e) => ui::warn(&format!("mcp '{}' connect failed: {e}", srv.name)),
        }
    }

    // Subagents need their own copy of the tool registry (Arc clones are cheap).
    let subagent_tools = tools;

    let store = SessionStore::open(&aether_config::Config::default_dir().join("sessions.db"))?;

    // Resume reuses the prior session id (and its persisted engineering state); otherwise we
    // start fresh (or adopt the id passed by `--background`).
    let (session_id, resume_plan): (String, Option<String>) = if let Some(id) = &cli.resume {
        let plan = store.get(id).ok().flatten().and_then(|m| m.plan);
        ui::note(&format!("resuming session {id}"));
        (id.clone(), plan)
    } else if let Some(id) = &cli.session_id {
        (id.clone(), None)
    } else {
        (store.new_session()?, None)
    };

    // Worktree isolation (Phase 4): run inside a git worktree so agent edits are reviewable
    // and never touch the user's main working tree until they merge.
    let base_cwd = std::env::current_dir()?;
    let run_cwd = if cli.worktree {
        make_worktree(&base_cwd)?
    } else {
        base_cwd.clone()
    };
    if cli.worktree {
        ui::note(&format!("working in git worktree: {}", run_cwd.display()));
    }
    ui::banner("aether");
    ui::note(&format!("session: {session_id}"));

    // ---- v0.12 subsystems ----
    let permission_engine = {
        let pe = aether_permissions::PermissionEngine::from_policy(&policy);
        // Add a sensible default deny on .env secrets.
        pe.add_global(
            aether_permissions::Rule::new(
                aether_permissions::Operation::Read,
                aether_permissions::ResourceScope::Glob { value: "**/.env".into() },
                aether_permissions::Permission::Deny,
            ).with_reason("secrets are not readable by default")
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
            // Fallback: open in a temp dir if the default can't be created.
            let tmp = std::env::temp_dir().join(format!("aether-snap-{session_id}"));
            aether_sessions::SnapshotManager::open(tmp).expect("snapshots dir")
        }),
    ));

    // ---- v0.13 subsystems ----
    let plugins = Arc::new(aether_plugin::Registry::new());
    let evidence = Arc::new(aether_evidence::EvidenceBag::new());
    let context_workspace = Arc::new(aether_context::ContextWorkspace::new());

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
        cfg.agent.cheap_model.clone(),
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
    .with_context_workspace(context_workspace);

    let format = |task: &str| -> String {
        if cli.plan {
            format!("{task}\n\n(PLAN MODE — read-only. Produce the structured PLAN document; do not modify files.)")
        } else {
            task.to_string()
        }
    };

    let mut current_mode: Mode = if cli.plan { Mode::Plan } else { Mode::Build };
    let mut last_plan: Option<String> = None;

    match cli.task.or(cli.prompt) {
        Some(task) => {
            let outcome = agent
                .run(&format(&task), current_mode, resume_plan.as_deref(), cli.resume.as_deref())
                .await?;
                if cli.json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "mode": current_mode.label(),
                            "plan": outcome.plan,
                            "result": outcome.result,
                            "review": outcome.review,
                            "test": outcome.test,
                            "engineering": outcome.engineering
                        })
                    );
                } else {
                    println!("\n{}\n", outcome.result);
                }
                if cli.traces {
                    print_traces(&store, &session_id);
                }
        }
        None => {
            if !std::io::stdin().is_terminal() {
                eprintln!("Usage: aether \"<task>\"");
                eprintln!("  Run from a terminal with a task, or just 'aether' for interactive mode.");
                return Ok(());
            }
            println!("aether — mode: {}. Type a task, /plan, /build, /mode, or /exit.", current_mode);
            let mut line = String::new();
            loop {
                print!("aether> ");
                std::io::stdout().flush()?;
                line.clear();
                if std::io::stdin().read_line(&mut line)? == 0 {
                    break;
                }
                let task = line.trim();
                if task.is_empty() {
                    continue;
                }
                if task == "/exit" || task == "/quit" {
                    break;
                }
                if task == "/plan" {
                    current_mode = Mode::Plan;
                    ui::note("switched to PLAN MODE (read-only)");
                    continue;
                }
                if task == "/build" {
                    current_mode = Mode::Build;
                    ui::note("switched to BUILD MODE");
                    continue;
                }
                if task == "/mode" {
                    println!("Current mode: {}\nAvailable modes: BUILD, PLAN", current_mode);
                    continue;
                }
                if task == "/traces" {
                    print_traces(&store, &session_id);
                    continue;
                }
                if task.starts_with("/resume") {
                    let id = task.split_whitespace().nth(1);
                    match id {
                        Some(id) => match store.get(id) {
                            Ok(Some(meta)) => {
                                let plan = meta.plan.clone();
                                let t = meta.task.clone().unwrap_or_else(|| task.to_string());
                                ui::note(&format!("resuming session {id}"));
                                match agent
                                    .run(&t, current_mode, plan.as_deref(), Some(id))
                                    .await
                                {
                                    Ok(o) => println!("\n{}\n", o.result),
                                    Err(e) => ui::error(&e.to_string()),
                                }
                            }
                            _ => ui::error("session not found"),
                        },
                        None => ui::warn("usage: /resume <session_id>"),
                    }
                    continue;
                }
                // In BUILD MODE, reuse a plan produced earlier in PLAN MODE (spec §22).
                let plan_arg: Option<String> = if current_mode.is_plan() { None } else { last_plan.clone() };
                match agent.run(&format(task), current_mode, plan_arg.as_deref(), None).await {
                    Ok(o) => {
                        if current_mode.is_plan() {
                            last_plan = Some(o.plan.clone());
                        }
                        println!("\n{}\n", o.result);
                    }
                    Err(e) => ui::error(&e.to_string()),
                }
            }
        }
    }

    Ok(())
}

/// Print a session's trace log (agent actions / decisions / verification) for debugging.
fn print_traces(store: &SessionStore, session_id: &str) {
    match store.list_traces(session_id, 200) {
        Ok(traces) if traces.is_empty() => ui::note("no traces recorded for this session"),
        Ok(traces) => {
            ui::section("[traces]", &format!("{} event(s)", traces.len()));
            for t in traces.iter().rev() {
                let when = t.ts.get(11..19).unwrap_or(&t.ts);
                println!("  {}  {:<9} {:<14} {}", when, t.kind, t.agent, t.summary);
            }
        }
        Err(e) => ui::error(&e.to_string()),
    }
}

/// Keep the console window open on error/exit when launched from Explorer (double-click),
/// where the window would otherwise close instantly and hide the message. Only pauses when
/// stdout is an actual terminal (so piped/CI usage is unaffected).
fn pause_if_terminal() {
    if std::io::stdout().is_terminal() {
        eprint!("Press Enter to exit...");
        let _ = std::io::stdout().flush();
        let mut s = String::new();
        let _ = std::io::stdin().read_line(&mut s);
    }
}

/// Print a friendly first-run / misconfiguration message instead of a bare error.
/// Create a git worktree (Phase 4) so the agent's edits are isolated from the user's main
/// working tree. Returns the worktree path; the caller runs the agent there. The worktree and
/// its branch are left in place for the user to review and merge manually.
fn make_worktree(cwd: &Path) -> anyhow::Result<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()?;
    if !out.status.success() {
        anyhow::bail!("--worktree requires a git repository; this directory is not one");
    }
    let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let root_path = Path::new(&root);
    let repo_name = root_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    let short = &uuid::Uuid::new_v4().to_string()[..8];
    let parent = root_path.parent().unwrap_or_else(|| Path::new("."));
    let wt = parent.join(format!("{repo_name}-aether-{short}"));
    let branch = format!("aether/{short}");
    let status = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["worktree", "add", "--force", "-b", &branch])
        .arg(&wt)
        .arg("HEAD")
        .status()?;
    if !status.success() {
        anyhow::bail!("failed to create git worktree at {}", wt.display());
    }
    Ok(wt)
}

fn setup_help(cfg_path: &PathBuf, cfg_missing: bool) {
    eprintln!();
    eprintln!("AETHER needs a model configuration before it can run.");
    if cfg_missing {
        eprintln!("No config file was found at:");
        eprintln!("  {}", cfg_path.display());
        eprintln!("Create one (e.g. copy config.example.toml from the repo) with at least a");
        eprintln!("[models.controller] and [models.executor] section. Minimum example:");
    } else {
        eprintln!(
            "The config at {} is missing the '{}' and/or '{}' model.",
            cfg_path.display(),
            "controller",
            "executor"
        );
        eprintln!("Add a [models.controller] and [models.executor] section. Minimum example:");
    }
    eprintln!();
    eprintln!(
        r#"  [agent]
  controller_model = "controller"
  executor_model   = "executor"

  [models.controller]
  provider    = "openai_compatible"
  base_url    = "https://api.openai.com/v1"
  model       = "gpt-4o-mini"
  api_key_env = "OPENAI_API_KEY"

  [models.executor]
  provider    = "openai_compatible"
  base_url    = "https://api.openai.com/v1"
  model       = "gpt-4o"
  api_key_env = "OPENAI_API_KEY"#
    );
    eprintln!();
    eprintln!("Then set the API key via an ENVIRONMENT VARIABLE (AETHER never stores keys on disk):");
    eprintln!("  CMD:        set OPENAI_API_KEY=sk-...");
    eprintln!("  PowerShell:  $env:OPENAI_API_KEY = \"...\"");
    eprintln!();
    eprintln!("AETHER is a COMMAND-LINE tool, not a GUI app. Run it from a terminal, e.g.:");
    eprintln!("  aether \"explain the main loop in src/main.rs\"");
    eprintln!();
    pause_if_terminal();
}
