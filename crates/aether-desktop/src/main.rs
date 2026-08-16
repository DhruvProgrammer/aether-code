//! aether-desktop — Tauri backend.
//!
//! Wraps the existing `aether.exe` CLI in a windowed app. The frontend (HTML/TS)
//! calls the `#[tauri::command]`s registered here via Tauri's IPC.
//!
//! Responsibilities:
//!   * Read/write `~/.aether/config.toml` from the Settings screen.
//!   * List past sessions from `~/.aether/sessions.db` for the History sidebar.
//!   * Run a task by spawning the bundled `aether.exe` child process and
//!     streaming its stdout/stderr to the frontend via `task-output` events.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

fn aether_dir() -> PathBuf {
    if let Ok(p) = std::env::var("AETHER_DIR") {
        return PathBuf::from(p);
    }
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("aether");
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".aether");
    }
    PathBuf::from(".aether")
}

fn config_path() -> PathBuf { aether_dir().join("config.toml") }
fn sessions_db() -> PathBuf { aether_dir().join("sessions.db") }

// ---------------------------------------------------------------------------
// Config model (mirrors aether-config; duplicated here to keep the desktop
// crate independent of the rest of the workspace during `tauri build`).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DesktopConfig {
    #[serde(default)]
    agent: AgentBlock,
    #[serde(default)]
    models: HashMap<String, ModelBlock>,
    #[serde(default, rename = "frontend")]
    frontend: FrontendBlock,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AgentBlock {
    #[serde(default = "default_controller")]
    controller_model: String,
    #[serde(default = "default_executor")]
    executor_model: String,
    #[serde(default)]
    reviewer_model: Option<String>,
}

fn default_controller() -> String { "controller".into() }
fn default_executor() -> String { "executor".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelBlock {
    provider: String,
    base_url: String,
    model: String,
    #[serde(default = "default_api_key_env")]
    api_key_env: String,
    #[serde(default)]
    extra_body: Option<serde_json::Value>,
}

fn default_api_key_env() -> String { "OPENAI_API_KEY".into() }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FrontendBlock {
    #[serde(default)]
    capture_command: Option<String>,
    #[serde(default)]
    preview_command: Option<String>,
    #[serde(default = "default_max_visual")]
    max_visual_iterations: u32,
    #[serde(default)]
    force: bool,
}

fn default_max_visual() -> u32 { 5 }

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ConfigResponse {
    path: String,
    exists: bool,
    config: DesktopConfig,
}

#[tauri::command]
async fn read_config() -> Result<ConfigResponse, String> {
    let path = config_path();
    let exists = path.exists();
    let config = if exists {
        let txt = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        toml::from_str::<DesktopConfig>(&txt).unwrap_or_default()
    } else {
        DesktopConfig::default()
    };
    Ok(ConfigResponse {
        path: path.display().to_string(),
        exists,
        config,
    })
}

#[tauri::command]
async fn write_config(config: DesktopConfig) -> Result<String, String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body = toml::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[derive(Serialize)]
struct SessionRow {
    id: String,
    created_at: String,
    task: Option<String>,
    plan: Option<String>,
}

#[tauri::command]
async fn list_sessions() -> Result<Vec<SessionRow>, String> {
    let path = sessions_db();
    if !path.exists() {
        return Ok(Vec::new());
    }
    // Open the same SQLite file the CLI writes to and read the sessions table.
    let conn = rusqlite::Connection::open(&path).map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, created_at, task, plan FROM sessions ORDER BY created_at DESC LIMIT 200",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SessionRow {
                id: r.get(0)?,
                created_at: r.get(1)?,
                task: r.get(2)?,
                plan: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[derive(Serialize)]
struct MessageRow {
    role: String,
    content: String,
    ts: String,
}

#[tauri::command]
async fn get_session_messages(session_id: String) -> Result<Vec<MessageRow>, String> {
    let path = sessions_db();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = rusqlite::Connection::open(&path).map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT role, content, ts FROM messages WHERE session_id = ?1 ORDER BY id ASC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([&session_id], |r| {
            Ok(MessageRow {
                role: r.get(0)?,
                content: r.get(1)?,
                ts: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

#[derive(Serialize)]
struct RunHandle {
    session_id: String,
}

#[derive(Default)]
struct RunState {
    /// Map of session_id -> child PID for cancellation.
    running: Mutex<HashMap<String, u32>>,
}

#[tauri::command]
async fn run_task(
    app: AppHandle,
    state: State<'_, Arc<RunState>>,
    task: String,
    plan: Option<bool>,
) -> Result<RunHandle, String> {
    if task.trim().is_empty() {
        return Err("task is empty".into());
    }
    let plan = plan.unwrap_or(false);
    let exe = locate_aether_binary(&app).ok_or_else(|| {
        "could not locate aether-cli.exe. The aether desktop app needs the aether CLI to run tasks. \
         If you installed aether from the NSIS installer, the CLI should be at \
         C:\\Program Files\\aether\\aether-cli.exe. If you installed from a portable zip, place \
         aether.exe in the same directory as aether-desktop.exe, or add aether.exe to your PATH. \
         You can also set the AETHER_CLI_PATH environment variable to point at the CLI."
            .to_string()
    })?;

    let session_id = format!(
        "desktop-{}-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        uuid::Uuid::new_v4().simple()
    );

    let mut cmd = Command::new(&exe);
    cmd.arg(&task)
        .arg("--session-id")
        .arg(&session_id)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if plan {
        cmd.arg("--plan");
    }
    #[cfg(windows)]
    {
        cmd.creation_flags(0x00000008); // DETACHED_PROCESS
    }
    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
    let pid = child.id().unwrap_or(0);

    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;

    // Track this run so the frontend can cancel it.
    {
        let mut runs = state.running.lock().await;
        runs.insert(session_id.clone(), pid);
    }

    let app_out = app.clone();
    let sid_out = session_id.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = app_out.emit(
                "task-output",
                TaskOutput {
                    session_id: sid_out.clone(),
                    stream: "stdout".into(),
                    line,
                },
            );
        }
    });

    let app_err = app.clone();
    let sid_err = session_id.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = app_err.emit(
                "task-output",
                TaskOutput {
                    session_id: sid_err.clone(),
                    stream: "stderr".into(),
                    line,
                },
            );
        }
    });

    let app_done = app.clone();
    let sid_done = session_id.clone();
    let state_done = state.inner().clone();
    tokio::spawn(async move {
        let status = child.wait().await;
        {
            let mut runs = state_done.running.lock().await;
            runs.remove(&sid_done);
        }
        let _ = app_done.emit(
            "task-exit",
            TaskExit {
                session_id: sid_done,
                code: status.as_ref().map(|s| s.code()).unwrap_or(None),
                success: status.as_ref().map(|s| s.success()).unwrap_or(false),
            },
        );
    });

    Ok(RunHandle { session_id })
}

#[derive(Serialize, Clone)]
struct TaskOutput {
    session_id: String,
    stream: String,
    line: String,
}

#[derive(Serialize, Clone)]
struct TaskExit {
    session_id: String,
    code: Option<i32>,
    success: bool,
}

#[tauri::command]
async fn cancel_task(state: State<'_, Arc<RunState>>, session_id: String) -> Result<bool, String> {
    let mut runs = state.running.lock().await;
    if let Some(pid) = runs.remove(&session_id) {
        #[cfg(windows)]
        {
            // Best-effort terminate via taskkill.
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F", "/T"])
                .output()
                .await;
        }
        #[cfg(not(windows))]
        {
            let _ = nix_signal(pid);
        }
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(not(windows))]
fn nix_signal(_pid: u32) {}

// Locate the bundled `aether-cli.exe` (the CLI). On a Windows NSIS install
// the Tauri app exe is renamed to `aether.exe`, so the CLI ships under a
// distinct name (`aether-cli.exe`) to avoid filename collision. We probe every
// plausible location: parent of the running app exe, the Tauri resource
// directory, the `resources/` subdir of the install dir, and finally the
// system PATH. The `.exe`/`.com` suffix is added automatically by Tauri's
// resource resolver on Windows.
fn locate_aether_binary(app: &AppHandle) -> Option<PathBuf> {
    // 0. Explicit override via env var (escape hatch).
    if let Ok(p) = std::env::var("AETHER_CLI_PATH") {
        let cand = PathBuf::from(p);
        if cand.exists() {
            return Some(cand);
        }
    }

    // Order matters: check the install-root CLI name first (v0.9.1+), then the
    // generic name (in case the user dropped `aether.exe` alongside manually),
    // then PATH.
    let names = ["aether-cli", "aether-cli.exe", "aether", "aether.exe"];

    // 1. Next to the running Tauri app exe (typical NSIS install layout).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in &names {
                let cand = dir.join(name);
                if cand.exists() {
                    return Some(cand);
                }
            }
            // 2. <install-dir>/resources/ — Tauri 2 alternative resource layout.
            let res_dir = dir.join("resources");
            if res_dir.is_dir() {
                for name in &names {
                    let cand = res_dir.join(name);
                    if cand.exists() {
                        return Some(cand);
                    }
                }
            }
        }
    }

    // 3. Tauri's resource API (resolves under BaseDirectory::Resource).
    for name in &names {
        if let Ok(p) = app.path().resolve(name, tauri::path::BaseDirectory::Resource) {
            if p.exists() {
                return Some(p);
            }
        }
    }

    // 4. System PATH (handles dev mode where `aether` is on PATH).
    which::which("aether").ok()
}

#[tauri::command]
fn aether_dir_str() -> String { aether_dir().display().to_string() }

#[derive(Serialize)]
struct LocateResult {
    found: Option<String>,
    searched: Vec<String>,
}

#[tauri::command]
fn locate_cli(app: AppHandle) -> LocateResult {
    let mut searched = Vec::new();
    let names = ["aether-cli", "aether-cli.exe", "aether", "aether.exe"];

    if let Ok(p) = std::env::var("AETHER_CLI_PATH") {
        searched.push(format!("AETHER_CLI_PATH={p}"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in &names {
                searched.push(dir.join(name).display().to_string());
            }
            searched.push(dir.join("resources").display().to_string() + "/*");
        }
    }
    let found = locate_aether_binary(&app).map(|p| p.display().to_string());
    LocateResult { found, searched }
}

#[tauri::command]
fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    let state = Arc::new(RunState::default());
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(state)
        .setup(|_app| Ok(()))
        .invoke_handler(tauri::generate_handler![
            read_config,
            write_config,
            list_sessions,
            get_session_messages,
            run_task,
            cancel_task,
            aether_dir_str,
            version,
            locate_cli,
        ])
        .run(tauri::generate_context!())
        .expect("error while running aether-desktop");
}

// Keep a small reference so unused imports are not flagged on stripped targets.
const _: () = ();
