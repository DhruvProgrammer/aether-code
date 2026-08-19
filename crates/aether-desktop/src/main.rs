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

use aether_analysis::AnalysisProvider as _;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

mod background;

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
    #[serde(default)]
    appearance: AppearanceBlock,
}

/// The three model slots (spec §3). Each slot points to a key in `models`.
/// - `model1`: Required — Big Executor.
/// - `model2`: Optional — Small Controller.
/// - `model3`: Optional — Visual Frontend Reviewer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AgentBlock {
    #[serde(default = "default_executor")]
    executor_model: String,
    #[serde(default = "default_controller")]
    controller_model: String,
    #[serde(default)]
    reviewer_model: Option<String>,
    /// Slot-1 explicit key (preferred over `executor_model` when present).
    #[serde(default)]
    model1: Option<String>,
    /// Slot-2 explicit key (preferred over `controller_model` when present).
    #[serde(default)]
    model2: Option<String>,
    /// Slot-3 explicit key (preferred over `reviewer_model` when present).
    #[serde(default)]
    model3: Option<String>,
}

fn default_controller() -> String { "controller".into() }
fn default_executor() -> String { "executor".into() }

/// Custom OpenAI-compatible provider (spec §8).
///
/// Per spec §21 we expose ONLY:
///   * Provider ID
///   * Base URL
///   * API Key
///   * Models
///
/// Display Name and Headers are deliberately absent from this struct and from
/// the UI. `extra_body` is an internal-only hook (used by the OpenAI-compatible
/// provider to pass through extra JSON body fields) and is not surfaced.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelBlock {
    provider: String,
    base_url: String,
    model: String,
    #[serde(default = "default_api_key_env")]
    api_key_env: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

/// Appearance block (spec §11-§18). Persisted in `~/.aether/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppearanceBlock {
    #[serde(default = "default_bg_enabled")]
    background_enabled: bool,
    #[serde(default = "default_bg_opacity")]
    background_opacity: u8,
    /// Resolved path to the user-chosen background. `None` ⇒ use bundled default.
    #[serde(default)]
    background_image: Option<String>,
}

fn default_bg_enabled() -> bool { true }
fn default_bg_opacity() -> u8 { 60 }

impl Default for AppearanceBlock {
    fn default() -> Self {
        AppearanceBlock {
            background_enabled: default_bg_enabled(),
            background_opacity: default_bg_opacity(),
            background_image: None,
        }
    }
}

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
// v0.12 subsystems — Provider health / Snapshot / Skills
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// v0.15 Model Gateway — per-role live validation + fingerprint-gated save
// ---------------------------------------------------------------------------

/// Map the desktop's role ids to gateway roles.
fn parse_role(role: &str) -> Option<aether_gateway::Role> {
    match role {
        "executor" | "model1" => Some(aether_gateway::Role::Executor),
        "controller" | "model2" => Some(aether_gateway::Role::Controller),
        "reviewer" | "model3" => Some(aether_gateway::Role::Reviewer),
        _ => None,
    }
}

/// Validation outcome surfaced to the settings UI.
#[derive(Serialize)]
struct RoleValidationDto {
    role: String,
    ok: bool,
    class: Option<String>,
    detail: String,
    latency_ms: u64,
    fingerprint: Option<String>,
    validated_at: Option<String>,
}

/// Run a live API validation for one role's configured provider/model.
/// On success, records a fingerprint snapshot so Save/Activate can be gated.
#[tauri::command]
async fn gateway_validate_role(role: String, config: DesktopConfig) -> Result<RoleValidationDto, String> {
    let r = parse_role(&role).ok_or_else(|| format!("unknown role: {role}"))?;
    let key = role_key_for(&config, r)
        .ok_or_else(|| format!("no model key bound to role {role}"))?;
    let mb = config
        .models
        .get(&key)
        .ok_or_else(|| format!("model key '{key}' not found in [models]"))?;
    let target = aether_gateway::ValidateTarget {
        role: r,
        model_key: key,
        provider_id: mb.provider.clone(),
        base_url: mb.base_url.clone(),
        model_id: mb.model.clone(),
        api_key_env: mb.api_key_env.clone(),
        extra_body: mb.extra_body.clone(),
    };
    let out = aether_gateway::validate_binding(&target).await;
    if let Some(snapshot) = &out.snapshot {
        if let Ok(store) = aether_gateway::ValidationStore::default_path() {
            let mut store = store;
            let _ = store.record(snapshot.clone());
        }
    }
    Ok(RoleValidationDto {
        role: role.clone(),
        ok: out.ok,
        class: out.class.map(|c| c.as_str().to_string()),
        detail: out.detail,
        latency_ms: out.latency_ms,
        fingerprint: out.fingerprint,
        validated_at: out.snapshot.map(|s| s.validated_at),
    })
}

/// Validation state for Save/Activate gating, given the *current* form values.
/// If the stored snapshot fingerprint no longer matches the current config,
/// validation is considered stale (spec §11).
#[derive(Serialize)]
struct RoleStatusDto {
    role: String,
    model_key: String,
    valid: bool,
    reason: Option<String>,
    validated_at: Option<String>,
}

#[tauri::command]
async fn gateway_validation_status(role: String, config: DesktopConfig) -> Result<RoleStatusDto, String> {
    let r = parse_role(&role).ok_or_else(|| format!("unknown role: {role}"))?;
    let key = role_key_for(&config, r).unwrap_or_default();
    let current_fp = match config.models.get(&key) {
        Some(mb) => aether_gateway::fingerprint_binding(
            r,
            &mb.provider,
            &mb.base_url,
            &mb.model,
            &mb.api_key_env,
            mb.extra_body.as_ref(),
        ),
        None => String::new(),
    };
    let store = aether_gateway::ValidationStore::default_path()
        .map_err(|e| format!("cannot open validation store: {e}"))?;
    let st = store.status_for(r, &current_fp);
    Ok(RoleStatusDto {
        role: role.clone(),
        model_key: key,
        valid: st.valid,
        reason: st.reason,
        validated_at: st.snapshot.map(|s| s.validated_at),
    })
}

/// Which `[models]` key a role is bound to, honouring model1/model2/model3
/// with the legacy names as fallback (mirrors `GatewayBundle::resolve`).
fn role_key_for(config: &DesktopConfig, role: aether_gateway::Role) -> Option<String> {
    match role {
        aether_gateway::Role::Executor => config
            .agent
            .model1
            .clone()
            .or_else(|| Some(config.agent.executor_model.clone()))
            .filter(|k| !k.is_empty()),
        aether_gateway::Role::Controller => config
            .agent
            .model2
            .clone()
            .or_else(|| Some(config.agent.controller_model.clone()))
            .filter(|k| !k.is_empty()),
        aether_gateway::Role::Reviewer => config
            .agent
            .model3
            .clone()
            .or_else(|| config.agent.reviewer_model.clone()),
    }
}

/// Run a one-shot health check against a candidate provider descriptor.
/// Fields mirror `aether_registry::ProviderDescriptor`; the frontend builds
/// the descriptor from the settings form.
#[tauri::command]
async fn check_provider(
    base_url: String,
    api_key_env: String,
    models: Vec<String>,
) -> aether_registry::HealthOutcome {
    let p = aether_registry::ProviderDescriptor::new_openai_compatible("probe", base_url, api_key_env);
    let mut p = p;
    for m in models { p = p.with_model(m); }
    aether_registry::HealthChecker::new().check(&p).await
}

#[derive(Serialize)]
struct SkillSummaryDto {
    id: String,
    name: String,
    description: String,
    version: String,
    tags: Vec<String>,
    source_path: String,
}

/// List skills discovered from the user's home + workspace + bundled resources.
#[tauri::command]
async fn list_skills(app: AppHandle) -> Vec<SkillSummaryDto> {
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() { roots.push(home.join(".aether/skills")); }
    roots.push(std::path::PathBuf::from("."));
    if let Ok(p) = app.path().resolve("skills", tauri::path::BaseDirectory::Resource) {
        roots.push(p);
    }
    let mut reg = aether_mind::skills::SkillRegistry::new();
    for r in roots {
        let _ = reg.scan_more(&r, 5);
    }
    reg.register_bundled();
    reg.summaries().into_iter().map(|s| SkillSummaryDto {
        id: s.id, name: s.name, description: s.description, version: s.version, tags: s.tags, source_path: s.source_path.display().to_string(),
    }).collect()
}

#[derive(Serialize)]
struct SnapshotDto {
    id: String,
    parent_id: Option<String>,
    timestamp: String,
    trigger: String,
    agent_id: Option<String>,
    task: Option<String>,
    files: Vec<String>,
    metadata: std::collections::HashMap<String, String>,
}

#[tauri::command]
fn list_snapshots(session_id: String) -> Vec<SnapshotDto> {
    let root = aether_config::expand_tilde(&format!("~/.aether/snapshots/{}", session_id));
    let mgr = match aether_sessions::SnapshotManager::open(root) { Ok(m) => m, Err(_) => return vec![] };
    mgr.list(&session_id).into_iter().map(|s| SnapshotDto {
        id: s.id.clone(),
        parent_id: s.parent_id.clone(),
        timestamp: s.timestamp.to_rfc3339(),
        trigger: s.trigger.label().into(),
        agent_id: s.agent_id.clone(),
        task: s.task.clone(),
        files: s.files.iter().map(|f| f.path.display().to_string()).collect(),
        metadata: s.metadata.clone(),
    }).collect()
}

#[derive(Serialize)]
struct SnapshotResultDto {
    snapshot_id: String,
    files_restored: usize,
    success: bool,
    message: String,
}

#[tauri::command]
fn restore_snapshot(session_id: String, snapshot_id: String) -> SnapshotResultDto {
    let root = aether_config::expand_tilde(&format!("~/.aether/snapshots/{}", session_id));
    let mut mgr = match aether_sessions::SnapshotManager::open(root) {
        Ok(m) => m,
        Err(e) => return SnapshotResultDto { snapshot_id, files_restored: 0, success: false, message: e.to_string() },
    };
    match mgr.restore(&snapshot_id) {
        Ok(s) => SnapshotResultDto {
            snapshot_id: s.id,
            files_restored: s.files.len(),
            success: true,
            message: "restored".into(),
        },
        Err(e) => SnapshotResultDto {
            snapshot_id,
            files_restored: 0,
            success: false,
            message: e.to_string(),
        },
    }
}

#[tauri::command]
fn snapshot_undo(session_id: String) -> SnapshotResultDto {
    let root = aether_config::expand_tilde(&format!("~/.aether/snapshots/{}", session_id));
    let mut mgr = match aether_sessions::SnapshotManager::open(root) {
        Ok(m) => m,
        Err(e) => return SnapshotResultDto { snapshot_id: String::new(), files_restored: 0, success: false, message: e.to_string() },
    };
    match mgr.undo(&session_id) {
        Ok(s) => SnapshotResultDto {
            snapshot_id: s.id,
            files_restored: s.files.len(),
            success: true,
            message: "undone".into(),
        },
        Err(e) => SnapshotResultDto {
            snapshot_id: String::new(),
            files_restored: 0,
            success: false,
            message: e.to_string(),
        },
    }
}

#[tauri::command]
fn snapshot_redo(session_id: String) -> SnapshotResultDto {
    let root = aether_config::expand_tilde(&format!("~/.aether/snapshots/{}", session_id));
    let mut mgr = match aether_sessions::SnapshotManager::open(root) {
        Ok(m) => m,
        Err(e) => return SnapshotResultDto { snapshot_id: String::new(), files_restored: 0, success: false, message: e.to_string() },
    };
    match mgr.redo(&session_id) {
        Ok(s) => SnapshotResultDto {
            snapshot_id: s.id,
            files_restored: s.files.len(),
            success: true,
            message: "redone".into(),
        },
        Err(e) => SnapshotResultDto {
            snapshot_id: String::new(),
            files_restored: 0,
            success: false,
            message: e.to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// Background image (spec §11-§18)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct BackgroundPayload {
    /// Image bytes (PNG or JPEG). May be empty if even the bundled default
    /// could not be located — the renderer treats this as "no background".
    data_base64: String,
    /// "image/png" or "image/jpeg".
    content_type: String,
    /// True if the payload is the bundled default (not user-supplied).
    is_default: bool,
}

/// Returns the active background image. If the user has selected a custom
/// image, that file is read from disk. Otherwise the bundled default
/// (`resources/default-background.png`) is used. If both fail, an empty
/// payload is returned (the UI hides the background layer in that case).
#[tauri::command]
async fn get_background(app: AppHandle) -> Result<BackgroundPayload, String> {
    // User-chosen image first.
    let cfg_path = config_path();
    if let Ok(txt) = std::fs::read_to_string(&cfg_path) {
        if let Ok(parsed) = toml::from_str::<DesktopConfig>(&txt) {
            if let Some(p) = parsed.appearance.background_image.as_deref() {
                let path = PathBuf::from(p);
                if path.exists() {
                    if let Ok(bytes) = std::fs::read(&path) {
                        let ct = if path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg")).unwrap_or(false) {
                            "image/jpeg"
                        } else {
                            "image/png"
                        };
                        return Ok(BackgroundPayload {
                            data_base64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes),
                            content_type: ct.into(),
                            is_default: false,
                        });
                    }
                }
            }
        }
    }

    // Bundled default.
    let candidates = [
        "default-background.png",
        "resources/default-background.png",
    ];
    for name in candidates.iter() {
        if let Ok(p) = app.path().resolve(name, tauri::path::BaseDirectory::Resource) {
            if p.exists() {
                if let Ok(bytes) = std::fs::read(&p) {
                    return Ok(BackgroundPayload {
                        data_base64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes),
                        content_type: "image/png".into(),
                        is_default: true,
                    });
                }
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let cand = dir.join(name);
                if cand.exists() {
                    if let Ok(bytes) = std::fs::read(&cand) {
                        return Ok(BackgroundPayload {
                            data_base64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes),
                            content_type: "image/png".into(),
                            is_default: true,
                        });
                    }
                }
            }
        }
    }

    Ok(BackgroundPayload {
        data_base64: String::new(),
        content_type: "image/png".into(),
        is_default: true,
    })
}

#[derive(Serialize)]
struct BackgroundValidation {
    accepted: bool,
    message: String,
    width: u32,
    height: u32,
    saved_path: Option<String>,
}

/// Accepts raw image bytes (PNG or JPEG), validates dimensions against the
/// canonical AETHER resolution, and persists the image to
/// `~/.aether/background.png`. On rejection the image is NOT saved and a
/// human-readable error is returned (spec §14).
#[tauri::command]
async fn set_background_image(bytes: Vec<u8>) -> Result<BackgroundValidation, String> {
    let (w, h) = background::validate_dimensions_bytes(&bytes).map_err(|e| e.to_string())?;
    let dir = aether_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join("background.png");
    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
    Ok(BackgroundValidation {
        accepted: true,
        message: format!("Accepted: {} x {} px", w, h),
        width: w,
        height: h,
        saved_path: Some(dest.display().to_string()),
    })
}

#[tauri::command]
fn required_background_resolution() -> String {
    background::required_resolution_label()
}

// ---------------------------------------------------------------------------
// v0.14 code-analysis capability — SonarQube integration
// ---------------------------------------------------------------------------
//
// Deterministic static analysis exposed as a capability, not an LLM.
// `analysis_check` → availability probe; `analysis_run` → fetch/run scan and
// persist the report; `analysis_latest` / `analysis_diff` → stored results.
// Findings are advisory input for the controller; the UI shows status,
// severity distribution, affected files and top findings only.

#[derive(Serialize)]
struct AnalysisAvailabilityDto {
    available: bool,
    detail: String,
}

fn sonar_provider(base_url: Option<String>, token_env: Option<String>, mode: Option<String>) -> aether_analysis::SonarQubeProvider {
    let m = match mode.as_deref() {
        Some("scanner") => aether_analysis::SonarQubeMode::ScannerApi,
        _ => aether_analysis::SonarQubeMode::Api,
    };
    let cfg = aether_analysis::SonarQubeConfig {
        base_url: base_url
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("SONAR_HOST_URL").ok())
            .unwrap_or_else(|| "http://localhost:9000".into()),
        token_env: token_env
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "SONAR_TOKEN".into()),
        mode: m,
        ..Default::default()
    };
    aether_analysis::SonarQubeProvider::new(cfg)
}

#[tauri::command]
async fn analysis_check(base_url: Option<String>, token_env: Option<String>) -> AnalysisAvailabilityDto {
    let prov = sonar_provider(base_url, token_env, None);
    let av = prov.availability().await;
    AnalysisAvailabilityDto { available: av.available, detail: av.detail }
}

#[derive(Serialize)]
struct FindingDto {
    id: String,
    rule: String,
    severity: String,
    kind: String,
    message: String,
    path: String,
    start_line: u32,
    status: String,
    remediation: Option<String>,
}

#[derive(Serialize)]
struct AnalysisReportDto {
    id: String,
    provider: String,
    project: String,
    at: String,
    label: Option<String>,
    finding_count: usize,
    info: usize,
    low: usize,
    medium: usize,
    high: usize,
    blocker: usize,
    affected_files: Vec<String>,
    findings: Vec<FindingDto>,
}

fn sev_rank(s: &str) -> u8 {
    match s { "blocker" => 4, "high" => 3, "medium" => 2, "low" => 1, _ => 0 }
}

fn report_to_dto(r: &aether_analysis::AnalysisReport) -> AnalysisReportDto {
    let mut findings: Vec<FindingDto> = r.findings.iter().map(|f| FindingDto {
        id: f.id.clone(),
        rule: f.rule.clone(),
        severity: f.severity.to_string(),
        kind: f.kind.to_string(),
        message: f.message.clone(),
        path: f.location.path.clone(),
        start_line: f.location.start_line,
        status: f.status.clone(),
        remediation: f.remediation.clone(),
    }).collect();
    findings.sort_by(|a, b| sev_rank(&b.severity).cmp(&sev_rank(&a.severity)).then_with(|| a.path.cmp(&b.path)));
    AnalysisReportDto {
        id: r.id.clone(),
        provider: r.provider.clone(),
        project: r.project.clone(),
        at: r.at.clone(),
        label: r.label.clone(),
        finding_count: r.findings.len(),
        info: r.distribution.info,
        low: r.distribution.low,
        medium: r.distribution.medium,
        high: r.distribution.high,
        blocker: r.distribution.blocker,
        affected_files: r.affected_files.clone(),
        findings,
    }
}

#[derive(Serialize)]
struct AnalysisRunResult {
    success: bool,
    message: String,
    report: Option<AnalysisReportDto>,
}

/// Run (or fetch latest) SonarQube analysis for a project directory.
/// `scope` restricts included paths when supported. Emits progress events.
#[tauri::command]
async fn analysis_run(
    app: AppHandle,
    project_root: String,
    mode: Option<String>,
    base_url: Option<String>,
    token_env: Option<String>,
    scope: Option<Vec<String>>,
    label: Option<String>,
) -> Result<AnalysisRunResult, String> {
    let _ = app.emit("analysis-progress", serde_json::json!({ "stage": "probing" }));
    let prov = sonar_provider(base_url, token_env, mode.clone());
    let av = prov.availability().await;
    if !av.available {
        return Ok(AnalysisRunResult { success: false, message: av.detail, report: None });
    }
    let _ = app.emit("analysis-progress", serde_json::json!({ "stage": "analyzing" }));
    let mut req = aether_analysis::AnalysisRequest::new(&project_root);
    req.scope = scope.unwrap_or_default();
    req.label = label;
    let result = if mode.as_deref() == Some("scanner") {
        prov.analyze(&req).await
    } else {
        match prov.latest_findings(&req).await {
            Ok(Some(f)) => {
                let mut rep = aether_analysis::AnalysisReport::new(
                    prov.id(),
                    &aether_analysis::project_key(&req.project_root),
                    &req.project_root,
                    f,
                );
                rep.label = req.label.clone();
                Ok(rep)
            }
            Ok(None) => prov.analyze(&req).await,
            Err(e) => Err(e),
        }
    };
    match result {
        Ok(report) => {
            if let Ok(store) = aether_analysis::AnalysisStore::default_dir() {
                let _ = store.save(&report);
            }
            let _ = app.emit("analysis-progress", serde_json::json!({ "stage": "done", "findings": report.findings.len() }));
            Ok(AnalysisRunResult { success: true, message: format!("{} findings", report.findings.len()), report: Some(report_to_dto(&report)) })
        }
        Err(e) => {
            let _ = app.emit("analysis-progress", serde_json::json!({ "stage": "error", "message": e.to_string() }));
            Ok(AnalysisRunResult { success: false, message: e.to_string(), report: None })
        }
    }
}

#[tauri::command]
async fn analysis_latest(project: String) -> Result<Option<AnalysisReportDto>, String> {
    let store = aether_analysis::AnalysisStore::default_dir().map_err(|e| e.to_string())?;
    Ok(store.latest(&project).map_err(|e| e.to_string())?.as_ref().map(report_to_dto))
}

#[tauri::command]
async fn analysis_projects() -> Vec<String> {
    aether_analysis::AnalysisStore::default_dir()
        .map(|s| s.projects())
        .unwrap_or_default()
}

#[derive(Serialize)]
struct RegressionDto {
    fingerprint: String,
    old_severity: String,
    new_severity: String,
}

#[derive(Serialize)]
struct AnalysisDiffDto {
    resolved: Vec<String>,
    remaining: Vec<String>,
    introduced: Vec<String>,
    regressions: Vec<RegressionDto>,
    baseline_count: usize,
    current_count: usize,
}

/// Diff a stored baseline report against the latest (or a given current) report.
#[tauri::command]
async fn analysis_diff(
    project: String,
    baseline_report: String,
    current_report: Option<String>,
) -> Result<AnalysisDiffDto, String> {
    let store = aether_analysis::AnalysisStore::default_dir().map_err(|e| e.to_string())?;
    let baseline = store.load(&project, &baseline_report).map_err(|e| e.to_string())?;
    let current = match current_report {
        Some(id) => store.load(&project, &id).map_err(|e| e.to_string())?,
        None => store.latest(&project).map_err(|e| e.to_string())?
            .ok_or_else(|| "no current report".to_string())?,
    };
    let d = aether_analysis::diff(&baseline.findings, &current.findings, &baseline.id, &current.id);
    Ok(AnalysisDiffDto {
        resolved: d.resolved,
        remaining: d.remaining,
        introduced: d.introduced,
        regressions: d.regressions.into_iter().map(|(f, o, n)| RegressionDto {
            fingerprint: f,
            old_severity: o.to_string(),
            new_severity: n.to_string(),
        }).collect(),
        baseline_count: d.baseline_count,
        current_count: d.current_count,
    })
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
            get_background,
            set_background_image,
            required_background_resolution,
            check_provider,
            gateway_validate_role,
            gateway_validation_status,
            list_skills,
            list_snapshots,
            restore_snapshot,
            snapshot_undo,
            snapshot_redo,
            analysis_check,
            analysis_run,
            analysis_latest,
            analysis_projects,
            analysis_diff,
        ])
        .run(tauri::generate_context!())
        .expect("error while running aether-desktop");
}

// Keep a small reference so unused imports are not flagged on stripped targets.
const _: () = ();
