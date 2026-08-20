//! AETHER CLI — public library surface.
//!
//! The desktop embeds the same `run_task` the CLI uses, so the desktop does
//! NOT spawn a subprocess and does NOT depend on a bundled `aether-cli.exe`.
//! Terminal users still get the standalone `aether` / `aether-mcp` binaries.
//!
//! See `aether-cli/src/main.rs` for the CLI wrapper and
//! `aether-desktop/src/main.rs` for the in-process call site.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Notify;

pub mod run_task;

pub use run_task::{RunOptions, TaskEvent, run};

/// Re-exported core items for downstream binary crates.
pub use aether_config;
pub use aether_core;
pub use aether_gateway;
pub use aether_mind;
pub use aether_models;
pub use aether_permissions;
pub use aether_sessions;
pub use aether_tools;

/// Spawn a task in-process and stream its output through `sink`. Returns
/// when the agent loop ends (success, error, or cancellation).
pub async fn run_with_sink(
    opts: RunOptions,
    cancel: Arc<Notify>,
    sink: Arc<dyn Fn(TaskEvent) + Send + Sync>,
) -> anyhow::Result<()> {
    run_task::run(opts, cancel, sink).await
}

/// Build the runtime config path; used by both the CLI and the desktop.
pub fn default_config_path() -> PathBuf {
    aether_config::Config::default_path()
}
