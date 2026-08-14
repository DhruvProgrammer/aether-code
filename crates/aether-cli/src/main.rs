//! CLI entrypoint (spec §23, §24). OpenAI-compatible coding agent.
mod ui;

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
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
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if cli.debug {
        tracing_subscriber::fmt().with_env_filter("debug").init();
    } else {
        tracing_subscriber::fmt().with_env_filter("info").init();
    }

    let mut cfg = aether_config::Config::load(cli.config)?;

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

    let controller_cfg = cfg
        .model(&cfg.agent.controller_model)
        .with_context(|| format!("model '{}' not configured", cfg.agent.controller_model))?;

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
                        "test": outcome.test
                    })
                );
            } else {
                println!("\n{}\n", outcome.result);
            }
        }
        None => {
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
