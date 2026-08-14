//! CLI entrypoint (spec §23, §24). OpenAI-compatible coding agent.
mod ui;

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use aether_core::agent_loop::Agent;
use aether_models::ModelProvider;
use aether_mind::{Mind, skills::SkillIndex};
use aether_permissions::Permission;
use aether_sessions::SessionStore;
use aether_tools::Tool;

#[derive(Parser)]
#[command(name = "aether", version, about = "OpenAI-compatible coding agent")]
struct Cli {
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
    /// Path to config.toml (defaults to ~/.aether/config.toml).
    #[arg(long)]
    config: Option<PathBuf>,
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
    let session_id = store.new_session()?;

    let agent = Agent::new(
        controller,
        cfg.agent.controller_model.clone(),
        cfg.agent.executor_model.clone(),
        providers,
        Some(store.clone()),
        session_id,
        mind,
        embedder,
        cfg.memory.auto_extract,
        cfg.memory.memory_top_k,
        std::env::current_dir()?,
        policy,
        subagent_tools,
        cfg.subagents.enabled,
        cfg.subagents.reviewer_model.clone(),
        cfg.subagents.tester_model.clone(),
        cfg.agent.cheap_model.clone(),
        cfg.agent.max_iterations,
        cfg.context.max_tokens,
        cfg.agent.loop_budget,
    );

    let format = |task: &str| -> String {
        if cli.plan {
            format!("{task}\n\n(PLANNING MODE — do not modify files. Return a numbered plan only.)")
        } else {
            task.to_string()
        }
    };

    ui::banner("aether");
    match cli.task.or(cli.prompt) {
        Some(task) => {
            let outcome = agent.run(&format(&task)).await?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
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
        }
        None => {
            if !std::io::stdin().is_terminal() {
                eprintln!("Usage: aether \"<task>\"");
                eprintln!("  Run from a terminal with a task, or just 'aether' for interactive mode.");
                return Ok(());
            }
            println!("aether — type a task, or /exit to quit.");
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
                match agent.run(&format(task)).await {
                    Ok(o) => println!("\n{}\n", o.result),
                    Err(e) => ui::error(&e.to_string()),
                }
            }
        }
    }

    Ok(())
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
