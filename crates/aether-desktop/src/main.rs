//! aether-desktop — Tauri backend.
//!
//! Wraps the existing `aether.exe` CLI in a windowed app. The frontend (HTML/TS)
//! calls the `#[tauri::command]`s registered here via Tauri's IPC.
//!
//! Responsibilities:
//!   * Read/write `~/.aether/config.toml` from the Settings screen.
//!   * List past sessions from `~/.aether/sessions.db` for the History sidebar.
//!   * Run a task by calling the shared `aether-cli::run_task` library
//!     in-process. No subprocess, no visible CLI window, no PATH dependency.
//!     Output is streamed to the frontend via `task-output` events.

// Hide the console window on Windows release builds — this is a GUI app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use aether_analysis::AnalysisProvider as _;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
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
    /// Display mode: "fill" (cover), "fit" (contain), "stretch", "center".
    #[serde(default = "default_bg_mode")]
    background_mode: String,
}

fn default_bg_enabled() -> bool { true }
fn default_bg_opacity() -> u8 { 60 }
fn default_bg_mode() -> String { "fill".into() }

impl Default for AppearanceBlock {
    fn default() -> Self {
        AppearanceBlock {
            background_enabled: default_bg_enabled(),
            background_opacity: default_bg_opacity(),
            background_image: None,
            background_mode: default_bg_mode(),
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
    /// Map of session_id -> cancel handle. Notify wakes the agent loop,
    /// which returns at the next iteration boundary. Owned by the spawned
    /// task; the desktop keeps one entry per active run so the UI can cancel.
    running: Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
}

/// Run an AETHER task in-process. The agent loop is shared with the CLI
/// binary (`aether-cli::run_task`) so there is no subprocess to spawn, no
/// console window to hide, and no bundled `aether-cli.exe` to locate. The
/// `task-output` and `task-exit` events keep the existing frontend wire
/// format, so the UI does not change.
///
/// The agent loop holds non-`Send` state (rusqlite Connection, the optional
/// visual `CorrectionExecutor`), so we run it on a dedicated single-thread
/// tokio runtime in its own OS thread — mirroring the TUI pattern. Events
/// are bridged back to the Tauri runtime via an `mpsc::channel`.
#[tauri::command]
async fn run_task(
    app: AppHandle,
    state: State<'_, Arc<RunState>>,
    task: String,
    plan: Option<bool>,
    session_id: Option<String>,
    workspace_path: Option<String>,
    role_assignments_json: Option<String>,
) -> Result<RunHandle, String> {
    if task.trim().is_empty() {
        return Err("Task is empty.".into());
    }
    let plan = plan.unwrap_or(false);

    let session_id = session_id.unwrap_or_else(|| {
        format!(
            "desktop-{}-{}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S"),
            uuid::Uuid::new_v4().simple()
        )
    });

    let cancel = Arc::new(tokio::sync::Notify::new());
    {
        let mut runs = state.running.lock().await;
        runs.insert(session_id.clone(), cancel.clone());
    }

    // v0.17: resolve per-session role assignments + provider registry.
    let (providers, role_assignments) = match role_assignments_json {
        Some(json) if !json.is_empty() => {
            let assignments: aether_config::RoleAssignments =
                serde_json::from_str(&json).map_err(|e| format!("invalid role assignments: {e}"))?;
            let provs = providers_list()?;
            let entries: Vec<aether_config::ProviderEntry> = provs
                .into_iter()
                .map(|p| aether_config::ProviderEntry {
                    id: p.id,
                    display_name: p.display_name,
                    protocol: p.protocol,
                    base_url: p.base_url,
                    api_key_env: p.api_key_env,
                    headers: p.headers,
                    extra_body: p.extra_body,
                    models: p
                        .models
                        .into_iter()
                        .map(|m| aether_config::ModelEntry {
                            id: m.id,
                            display_name: m.display_name,
                            vision: m.vision,
                            tool_calling: m.tool_calling,
                            streaming: m.streaming,
                            context_window: m.context_window,
                            max_output_tokens: m.max_output_tokens,
                        })
                        .collect(),
                })
                .collect();
            (Some(entries), Some(assignments))
        }
        _ => (None, None),
    };

    let opts = aether_cli::run_task::RunOptions {
        task: Some(task),
        plan,
        session_id: Some(session_id.clone()),
        workspace_path: workspace_path.map(std::path::PathBuf::from),
        providers,
        role_assignments,
        ..Default::default()
    };

    // Bridge: a dedicated thread runs the agent (single-thread tokio runtime
    // because `Agent::run` holds non-Send state). Events flow through a
    // tokio mpsc channel; the Tauri side has a `tokio::spawn` that drains
    // the channel and emits Tauri events on this runtime.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<TaskEventBridge>(64);
    let tx_for_sink = tx.clone();
    let cancel_for_thread = cancel.clone();
    let app_for_bridge = app.clone();
    let sid_for_bridge = session_id.clone();
    let state_for_bridge = state.inner().clone();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                let _ = tx_for_sink.blocking_send(TaskEventBridge::Failed(format!(
                    "backend runtime init failed: {e}"
                )));
                return;
            }
        };
        let sink: Arc<dyn Fn(aether_cli::run_task::TaskEvent) + Send + Sync> =
            Arc::new(move |e| {
                let _ = tx_for_sink.blocking_send(TaskEventBridge::Agent(e));
            });
        let outcome = rt.block_on(async move {
            aether_cli::run_task::run(opts, cancel_for_thread, sink).await
        });
        let exit = match outcome {
            Ok(()) => TaskEventBridge::Exit { code: 0, success: true },
            Err(e) => TaskEventBridge::Failed(format!("backend failed: {e}")),
        };
        let _ = tx.blocking_send(exit);
    });

    // Drain events on the Tauri runtime and emit Tauri events.
    let app_emit = app_for_bridge.clone();
    let sid_emit = sid_for_bridge.clone();
    let state_emit = state_for_bridge.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                TaskEventBridge::Agent(aether_cli::run_task::TaskEvent::Line {
                    stream,
                    line,
                }) => {
                    let _ = app_emit.emit(
                        "task-output",
                        TaskOutput {
                            session_id: sid_emit.clone(),
                            stream: stream.to_string(),
                            line,
                        },
                    );
                }
                TaskEventBridge::Agent(aether_cli::run_task::TaskEvent::Error { message }) => {
                    let _ = app_emit.emit(
                        "task-output",
                        TaskOutput {
                            session_id: sid_emit.clone(),
                            stream: "stderr".into(),
                            line: format!("error: {message}"),
                        },
                    );
                }
                TaskEventBridge::Agent(aether_cli::run_task::TaskEvent::TaskState { json }) => {
                    let _ = app_emit.emit("task-state", TaskStateEvent {
                        session_id: sid_emit.clone(),
                        payload: json,
                    });
                }
                TaskEventBridge::Agent(aether_cli::run_task::TaskEvent::Exit { .. }) => {}
                TaskEventBridge::Exit { code, success } => {
                    {
                        let mut runs = state_emit.running.lock().await;
                        runs.remove(&sid_emit);
                    }
                    let _ = app_emit.emit(
                        "task-exit",
                        TaskExit {
                            session_id: sid_emit.clone(),
                            code: Some(code),
                            success,
                        },
                    );
                    break;
                }
                TaskEventBridge::Failed(msg) => {
                    {
                        let mut runs = state_emit.running.lock().await;
                        runs.remove(&sid_emit);
                    }
                    let _ = app_emit.emit(
                        "task-output",
                        TaskOutput {
                            session_id: sid_emit.clone(),
                            stream: "stderr".into(),
                            line: msg.clone(),
                        },
                    );
                    let _ = app_emit.emit(
                        "task-exit",
                        TaskExit {
                            session_id: sid_emit.clone(),
                            code: Some(1),
                            success: false,
                        },
                    );
                    break;
                }
            }
        }
    });

    Ok(RunHandle { session_id })
}

/// Internal bridge payload between the agent thread and the Tauri runtime.
enum TaskEventBridge {
    Agent(aether_cli::run_task::TaskEvent),
    Exit { code: i32, success: bool },
    Failed(String),
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

#[derive(Serialize, Clone)]
struct TaskStateEvent {
    session_id: String,
    payload: String,
}

/// Cancel a running task. The agent loop checks the cancel handle at every
/// iteration boundary and returns with a `[cancelled by caller]` result.
#[tauri::command]
async fn cancel_task(state: State<'_, Arc<RunState>>, session_id: String) -> Result<bool, String> {
    let mut runs = state.running.lock().await;
    if let Some(handle) = runs.remove(&session_id) {
        handle.notify_waiters();
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
fn aether_dir_str() -> String { aether_dir().display().to_string() }

/// Returns the agent's runtime status. Replaces the legacy `/locate` slash
/// command — the desktop now drives the agent in-process, so there is no
/// bundled CLI binary to locate.
#[derive(Serialize)]
struct AgentStatus {
    /// Where the on-disk config is loaded from.
    config_path: String,
    /// Whether it exists. If false, the user needs to configure the desktop.
    config_exists: bool,
    /// AETHER backend version.
    version: String,
    /// Internal architecture marker.
    backend: &'static str,
}

#[tauri::command]
fn backend_status() -> AgentStatus {
    let path = config_path();
    AgentStatus {
        config_path: path.display().to_string(),
        config_exists: path.exists(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        backend: "in-process shared library (aether-cli::run_task)",
    }
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
        headers: None,
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
            None,
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

/// Accepts raw image bytes (PNG or JPEG), validates that they decode as a
/// real image within practical size limits (any resolution is accepted), and
/// persists the original file unchanged to `~/.aether/background.png`.
#[tauri::command]
async fn set_background_image(bytes: Vec<u8>) -> Result<BackgroundValidation, String> {
    let (w, h) = background::validate_bytes(&bytes).map_err(|e| e.to_string())?;
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
// v0.17 Workspace / Provider / Session architecture
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct WorkspaceDto {
    id: String,
    path: String,
    name: String,
    created_at: String,
    last_opened: String,
    last_session: Option<String>,
}

fn workspace_store() -> Result<aether_workspace::WorkspaceStore, String> {
    let path = aether_workspace::WorkspaceStore::default_path().map_err(|e| e.to_string())?;
    aether_workspace::WorkspaceStore::open(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn workspace_list(limit: Option<usize>) -> Result<Vec<WorkspaceDto>, String> {
    let store = workspace_store()?;
    let ws = store.recent(limit.unwrap_or(20)).map_err(|e| e.to_string())?;
    Ok(ws.into_iter().map(|w| WorkspaceDto {
        id: w.id, path: w.path, name: w.name,
        created_at: w.created_at, last_opened: w.last_opened, last_session: w.last_session,
    }).collect())
}

#[tauri::command]
fn workspace_open_folder(path: String) -> Result<WorkspaceDto, String> {
    let mut store = workspace_store()?;
    let w = store.open_folder(std::path::Path::new(&path)).map_err(|e| e.to_string())?;
    Ok(WorkspaceDto {
        id: w.id, path: w.path, name: w.name,
        created_at: w.created_at, last_opened: w.last_opened, last_session: w.last_session,
    })
}

#[tauri::command]
fn pick_folder() -> Result<Option<String>, String> {
    let picked = rfd::FileDialog::new()
        .set_title("Select Workspace Folder")
        .pick_folder();
    Ok(picked.map(|p| p.display().to_string()))
}

#[tauri::command]
fn workspace_remove(id: String) -> Result<(), String> {
    let store = workspace_store()?;
    store.remove(&id).map_err(|e| e.to_string())
}

#[tauri::command]
fn workspace_set_last_session(workspace_id: String, session_id: String) -> Result<(), String> {
    let store = workspace_store()?;
    store.set_last_session(&workspace_id, &session_id).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct SessionRowDto {
    id: String,
    created_at: String,
    task: Option<String>,
    title: Option<String>,
}

#[tauri::command]
fn workspace_sessions(workspace_id: String, limit: Option<usize>) -> Result<Vec<SessionRowDto>, String> {
    let store = aether_sessions::SessionStore::open(
        &aether_config::Config::default_dir().join("sessions.db"),
    ).map_err(|e| e.to_string())?;
    let sessions = store.list_by_workspace(&workspace_id, limit.unwrap_or(50)).map_err(|e| e.to_string())?;
    Ok(sessions.into_iter().map(|s| SessionRowDto {
        id: s.id, created_at: s.created_at, task: s.task, title: None,
    }).collect())
}

#[tauri::command]
fn workspace_create_session(workspace_id: String, title: Option<String>) -> Result<String, String> {
    let store = aether_sessions::SessionStore::open(
        &aether_config::Config::default_dir().join("sessions.db"),
    ).map_err(|e| e.to_string())?;
    let id = store.new_session_in_workspace(&workspace_id, title.as_deref()).map_err(|e| e.to_string())?;
    let ws_store = workspace_store()?;
    let _ = ws_store.set_last_session(&workspace_id, &id);
    Ok(id)
}

#[tauri::command]
fn session_set_roles(session_id: String, assignments_json: String) -> Result<(), String> {
    let store = aether_sessions::SessionStore::open(
        &aether_config::Config::default_dir().join("sessions.db"),
    ).map_err(|e| e.to_string())?;
    store.set_role_assignments(&session_id, &assignments_json).map_err(|e| e.to_string())
}

#[tauri::command]
fn session_get_roles(session_id: String) -> Result<Option<String>, String> {
    let store = aether_sessions::SessionStore::open(
        &aether_config::Config::default_dir().join("sessions.db"),
    ).map_err(|e| e.to_string())?;
    store.get_role_assignments(&session_id).map_err(|e| e.to_string())
}

#[derive(Serialize, Deserialize, Clone)]
struct ProviderEntryDto {
    id: String,
    display_name: String,
    protocol: String,
    base_url: String,
    api_key_env: String,
    #[serde(default)]
    headers: Option<serde_json::Value>,
    #[serde(default)]
    extra_body: Option<serde_json::Value>,
    #[serde(default)]
    models: Vec<ModelEntryDto>,
}

#[derive(Serialize, Deserialize, Clone)]
struct ModelEntryDto {
    id: String,
    display_name: String,
    #[serde(default)]
    vision: bool,
    #[serde(default = "default_true")]
    tool_calling: bool,
    #[serde(default = "default_true")]
    streaming: bool,
    #[serde(default)]
    context_window: Option<u32>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
}

fn default_true() -> bool { true }

fn providers_path() -> std::path::PathBuf {
    aether_config::Config::default_dir().join("providers.json")
}

#[tauri::command]
fn providers_list() -> Result<Vec<ProviderEntryDto>, String> {
    let path = providers_path();
    if !path.exists() {
        return Ok(vec![]);
    }
    let txt = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let providers: Vec<ProviderEntryDto> = serde_json::from_str(&txt).unwrap_or_default();
    Ok(providers)
}

#[tauri::command]
fn providers_save(providers: Vec<ProviderEntryDto>) -> Result<(), String> {
    let path = providers_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&providers).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

#[tauri::command]
async fn providers_validate(provider_id: String, model_id: String) -> Result<aether_gateway::validate::ValidationOutcome, String> {
    let providers = providers_list()?;
    let prov = providers.iter().find(|p| p.id == provider_id)
        .ok_or_else(|| format!("provider '{provider_id}' not found"))?;
    // Allow validation of a model_id that is not yet in the provider's model list (e.g. Add Model flow).
    // The model list is just UI state; validation should succeed/fail based on the provider's actual API response.
    let target = aether_gateway::ValidateTarget {
        role: aether_gateway::Role::Executor,
        model_key: format!("{provider_id}/{model_id}"),
        provider_id: prov.id.clone(),
        base_url: prov.base_url.clone(),
        model_id: model_id.clone(),
        api_key_env: prov.api_key_env.clone(),
        headers: prov.headers.clone(),
        extra_body: prov.extra_body.clone(),
    };
    Ok(aether_gateway::validate_binding(&target).await)
}

#[tauri::command]
async fn provider_check_connection(provider_id: String) -> Result<aether_registry::HealthOutcome, String> {
    let providers = providers_list()?;
    let prov = providers.iter().find(|p| p.id == provider_id)
        .ok_or_else(|| format!("provider '{provider_id}' not found"))?;
    if prov.base_url.trim().is_empty() {
        return Err("Base URL is empty — configure it before checking connection".into());
    }
    if prov.api_key_env.trim().is_empty() {
        return Err("API Key env is empty — configure it before checking connection".into());
    }
    let mut desc = aether_registry::ProviderDescriptor::new_openai_compatible(
        prov.id.clone(),
        prov.base_url.clone(),
        prov.api_key_env.clone(),
    );
    for m in &prov.models {
        desc = desc.with_model(m.id.clone());
    }
    Ok(aether_registry::HealthChecker::new().check(&desc).await)
}

/// Migrate the legacy `[agent]` model1/model2/model3 + `[models]` map into the
/// v0.17 provider registry. Runs once; idempotent (skips if providers.json
/// already exists). Preserves credential env-var references — never the keys.
#[tauri::command]
fn migrate_legacy_models() -> Result<u32, String> {
    let path = providers_path();
    if path.exists() {
        return Ok(0);
    }
    let cfg = aether_config::Config::load(None).map_err(|e| e.to_string())?;
    let mut providers: Vec<ProviderEntryDto> = Vec::new();
    let mut migrated = 0u32;

    for (key, mc) in &cfg.models {
        let prov_id = if mc.provider.is_empty() { key.clone() } else { mc.provider.clone() };
        let entry = providers.iter_mut().find(|p| p.id == prov_id);
        match entry {
            Some(p) => {
                if !p.models.iter().any(|m| m.id == mc.model) {
                    p.models.push(ModelEntryDto {
                        id: mc.model.clone(),
                        display_name: mc.model.clone(),
                        vision: false,
                        tool_calling: true,
                        streaming: true,
                        context_window: None,
                        max_output_tokens: None,
                    });
                }
            }
            None => {
                providers.push(ProviderEntryDto {
                    id: prov_id.clone(),
                    display_name: prov_id.clone(),
                    protocol: if mc.provider.is_empty() { "openai_compatible".into() } else { mc.provider.clone() },
                    base_url: mc.base_url.clone(),
                    api_key_env: mc.api_key_env.clone(),
                    headers: None,
                    extra_body: mc.extra_body.clone(),
                    models: vec![ModelEntryDto {
                        id: mc.model.clone(),
                        display_name: mc.model.clone(),
                        vision: false,
                        tool_calling: true,
                        streaming: true,
                        context_window: None,
                        max_output_tokens: None,
                    }],
                });
            }
        }
        migrated += 1;
    }

    if !providers.is_empty() {
        providers_save(providers)?;
    }
    Ok(migrated)
}

// ---------------------------------------------------------------------------
// Session compaction (v0.18)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct CompactResultDto {
    status: String,
    tokens_before: u32,
    tokens_after: u32,
    message: String,
}

/// Manually compact a session's context (`/compact`). Uses the session's
/// configured Model 2 (controller) to generate a structured checkpoint.
/// The durable message history is never deleted; only the active context is
/// reduced. Model assignments are never modified.
#[tauri::command]
async fn compact_session(session_id: String) -> Result<CompactResultDto, String> {
    // Load messages + resolve the controller binding BEFORE any await so the
    // non-Sync SessionStore is never held across an await point.
    let (messages, tokens_before, controller, controller_model, context_window) = {
        let store = aether_sessions::SessionStore::open(&sessions_db()).map_err(|e| e.to_string())?;
        let rows = store.get_messages(&session_id, 10_000).map_err(|e| e.to_string())?;
        if rows.is_empty() {
            return Ok(CompactResultDto {
                status: "idle".into(),
                tokens_before: 0,
                tokens_after: 0,
                message: "No messages to compact.".into(),
            });
        }
        let messages: Vec<aether_models::Message> = rows
            .iter()
            .map(|r| aether_models::Message {
                role: r.role.clone(),
                content: r.content.clone(),
                ..Default::default()
            })
            .collect();
        let tokens_before = aether_context::checkpoint::estimate_message_tokens(&messages);
        let (controller, controller_model, context_window) =
            resolve_session_controller(&session_id)?;
        (messages, tokens_before, controller, controller_model, context_window)
    };

    let compactor = aether_context::SessionCompactor::new(
        controller,
        controller_model,
        context_window,
        Arc::new(aether_core::compaction_store::SessionCheckpointStore::new(sessions_db())),
    );

    match compactor
        .compact(&session_id, "AETHER session compaction", &messages, aether_context::CompactTrigger::Manual)
        .await
    {
        Ok(rebuilt) => {
            let tokens_after = aether_context::checkpoint::estimate_message_tokens(&rebuilt);
            Ok(CompactResultDto {
                status: "completed".into(),
                tokens_before,
                tokens_after,
                message: "Context compacted".into(),
            })
        }
        Err(e) => Ok(CompactResultDto {
            status: "failed".into(),
            tokens_before,
            tokens_after: tokens_before,
            message: format!("Compaction failed (previous state kept): {e}"),
        }),
    }
}

/// Resolve the session's configured Model 2 (controller) provider + model +
/// context window from the provider registry and per-session role assignments.
/// Falls back to the global config when no session binding exists.
fn resolve_session_controller(
    session_id: &str,
) -> Result<(Arc<dyn aether_models::ModelProvider>, String, u32), String> {
    let cfg = aether_config::Config::load(None).map_err(|e| e.to_string())?;
    let context_window = cfg.context.max_tokens;

    // Try per-session role assignments first (v0.17 architecture).
    let store = aether_sessions::SessionStore::open(&sessions_db()).map_err(|e| e.to_string())?;
    if let Some(json) = store.get_role_assignments(session_id).map_err(|e| e.to_string())? {
        if let Ok(assignments) = serde_json::from_str::<aether_config::RoleAssignments>(&json) {
            if let Some(ctrl) = &assignments.controller {
                let providers = providers_list()?;
                if let Some(prov) = providers.iter().find(|p| p.id == ctrl.provider_id) {
                    if prov.models.iter().any(|m| m.id == ctrl.model_id) {
                        let window = prov
                            .models
                            .iter()
                            .find(|m| m.id == ctrl.model_id)
                            .and_then(|m| m.context_window)
                            .unwrap_or(context_window);
                        let mc = aether_config::ModelConfig {
                            provider: prov.protocol.clone(),
                            base_url: prov.base_url.clone(),
                            model: ctrl.model_id.clone(),
                            api_key_env: prov.api_key_env.clone(),
                            headers: prov.headers.clone(),
                            extra_body: prov.extra_body.clone(),
                        };
                        let provider = aether_models::build_provider(&mc).map_err(|e| e.to_string())?;
                        return Ok((Arc::from(provider), mc.model.clone(), window));
                    }
                }
            }
        }
    }

    // Fall back to the global controller model.
    let key = cfg.agent.model2.clone().unwrap_or_else(|| cfg.agent.controller_model.clone());
    let mc = cfg
        .model(&key)
        .ok_or_else(|| format!("controller model '{key}' not found in config"))?;
    let provider = aether_models::build_provider(mc).map_err(|e| e.to_string())?;
    Ok((Arc::from(provider), mc.model.clone(), context_window))
}

// ---------------------------------------------------------------------------
// Task state machine (v0.19)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct TaskStateDto {
    session_id: String,
    state: String,
    active_role: String,
    activity: String,
    plan_step: Option<u32>,
    total_steps: Option<u32>,
    active_tool: Option<String>,
    attempt_count: u32,
    repair_attempt_count: u32,
    replan_count: u32,
    verification_attempt_count: u32,
    last_error: Option<String>,
    next_action: Option<String>,
    transitions_len: usize,
}

#[tauri::command]
async fn get_task_state(session_id: String) -> Result<Option<TaskStateDto>, String> {
    let path = sessions_db();
    if !path.exists() {
        return Ok(None);
    }
    let store = aether_sessions::SessionStore::open(&path).map_err(|e| e.to_string())?;
    let json = store
        .get_kv(&session_id, "task_state")
        .map_err(|e| e.to_string())?;
    match json {
        Some(j) => {
            let tsm = aether_core::task_state::TaskStateMachine::deserialize(&j)
                .ok_or("failed to parse task state")?;
            let r = &tsm.record;
            Ok(Some(TaskStateDto {
                session_id: r.session_id.clone(),
                state: r.state.label().to_string(),
                active_role: r.active_role.label().to_string(),
                activity: r.current_activity.clone(),
                plan_step: r.current_plan_step,
                total_steps: r.total_plan_steps,
                active_tool: r.active_tool.clone(),
                attempt_count: r.attempt_count,
                repair_attempt_count: r.repair_attempt_count,
                replan_count: r.replan_count,
                verification_attempt_count: r.verification_attempt_count,
                last_error: r.last_error.clone(),
                next_action: r.next_action.clone(),
                transitions_len: r.transitions.len(),
            }))
        }
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    let state = Arc::new(RunState::default());
    tauri::Builder::default()
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
            backend_status,
            get_background,
            set_background_image,
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
            workspace_list,
            workspace_open_folder,
            pick_folder,
            workspace_remove,
            workspace_set_last_session,
            workspace_sessions,
            workspace_create_session,
            session_set_roles,
            session_get_roles,
            providers_list,
            providers_save,
            providers_validate,
            provider_check_connection,
            migrate_legacy_models,
            compact_session,
            get_task_state,
        ])
        .run(tauri::generate_context!())
        .expect("error while running aether-desktop");
}

// Keep a small reference so unused imports are not flagged on stripped targets.
const _: () = ();
