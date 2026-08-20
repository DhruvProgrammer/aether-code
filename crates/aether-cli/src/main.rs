//! CLI entrypoint (spec §23, §24). OpenAI-compatible coding agent.
//!
//! The actual agent-construction + run logic lives in [`aether_cli::run_task`].
//! The CLI binary is a thin wrapper: parse args, dispatch to the TUI when
//! interactive, otherwise forward output to stdout. The desktop embeds the
//! same `run_task` directly — no subprocess is spawned.

mod mcp;
mod tui;
mod ui;

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::sync::Notify;

use aether_cli::run_task::{run, RunOptions, TaskEvent};

#[derive(Parser, Clone)]
#[command(name = "aether", version, about = "OpenAI-compatible coding agent")]
pub struct Cli {
    /// Task to run non-interactively.
    pub task: Option<String>,
    /// Task to run non-interactively (explicit flag).
    #[arg(long)]
    pub prompt: Option<String>,
    #[arg(long)]
    pub plan: bool,
    #[arg(long)]
    pub local: bool,
    #[arg(long)]
    pub rollback: Option<String>,
    #[arg(long)]
    pub debug: bool,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub traces: bool,
    #[arg(long)]
    pub resume: Option<String>,
    #[arg(long)]
    pub worktree: bool,
    /// Run a task as a detached background process.
    #[arg(long)]
    pub background: Option<String>,
    #[arg(long, hide = true)]
    pub session_id: Option<String>,
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub tui: bool,
}

impl Cli {
    pub fn to_run_options(&self) -> RunOptions {
        RunOptions {
            task: self.task.clone(),
            prompt: self.prompt.clone(),
            plan: self.plan,
            local: self.local,
            rollback: self.rollback.clone(),
            debug: self.debug,
            json: self.json,
            traces: self.traces,
            resume: self.resume.clone(),
            worktree: self.worktree,
            background: self.background.clone(),
            session_id: self.session_id.clone(),
            config: self.config.clone(),
            tui: self.tui,
        }
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run_cli(cli).await {
        eprintln!("AETHER error: {e}");
        pause_if_terminal();
        std::process::exit(1);
    }
}

async fn run_cli(cli: Cli) -> anyhow::Result<()> {
    if cli.debug {
        tracing_subscriber::fmt().with_env_filter("debug").init();
    } else {
        tracing_subscriber::fmt().with_env_filter("info").init();
    }

    // TUI dispatch: when the user runs `aether` with no task on a real TTY.
    let has_task = cli.task.is_some() || cli.prompt.is_some();
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if cli.tui
        || (interactive
            && !has_task
            && cli.background.is_none()
            && cli.resume.is_none()
            && cli.rollback.is_none()
            && !cli.traces)
    {
        return tui::run_tui(cli, std::env::args().collect()).await;
    }

    let cancel = Arc::new(Notify::new());
    let opts = cli.to_run_options();
    let sink: Arc<dyn Fn(TaskEvent) + Send + Sync> = Arc::new(|e| match e {
        TaskEvent::Line { stream, line } => {
            if stream == "stderr" {
                let _ = std::io::stderr().write_all(line.as_bytes());
                let _ = std::io::stderr().write_all(b"\n");
            } else {
                let _ = std::io::stdout().write_all(line.as_bytes());
                let _ = std::io::stdout().write_all(b"\n");
            }
        }
        TaskEvent::Exit { .. } | TaskEvent::Error { .. } => {}
    });
    run(opts, cancel, sink).await
}

fn pause_if_terminal() {
    if std::io::stdin().is_terminal() {
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(b"\nPress Enter to exit...");
        let _ = stdout.flush();
        let _ = std::io::stdin().read_line(&mut String::new());
    }
}
