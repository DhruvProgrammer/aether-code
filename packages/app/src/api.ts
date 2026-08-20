// Thin wrapper over Tauri's invoke + event APIs.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ConfigResponse {
  path: string;
  exists: boolean;
  config: DesktopConfig;
}

/**
 * The three model slots (spec §3).
 * - Model 1 (model1): Required — Big Executor.
 * - Model 2 (model2): Optional — Small Controller.
 * - Model 3 (model3): Optional — Visual Reviewer.
 *
 * Each slot is a key into `models` (the OpenAI-compatible providers map).
 */
export interface DesktopConfig {
  agent: {
    /** Slot 1 — Big Executor (required). */
    model1: string;
    /** Slot 2 — Small Controller (optional; empty string ⇒ disabled). */
    model2: string;
    /** Slot 3 — Visual Reviewer (optional; null ⇒ disabled). */
    model3: string | null;
    // Legacy fields kept on disk for back-compat; the UI only edits `model1/2/3`.
    controller_model?: string;
    executor_model?: string;
    reviewer_model?: string | null;
  };
  /** OpenAI-compatible providers — each key is referenced by the three model slots. */
  models: Record<string, ModelBlock>;
  frontend: {
    capture_command: string | null;
    preview_command: string | null;
    max_visual_iterations: number;
    force: boolean;
  };
  appearance: AppearanceConfig;
}

/**
 * Custom OpenAI-compatible provider (spec §8).
 *
 * Only the four fields the spec mandates:
 *   - Provider ID
 *   - Base URL
 *   - API Key
 *   - Models
 *
 * Display Name and Headers are deliberately absent.
 */
export interface ModelBlock {
  provider: string;
  base_url: string;
  model: string;
  api_key_env: string;
}

export interface AppearanceConfig {
  background_enabled: boolean;
  background_opacity: number; // 0..100
  background_image: string | null; // resolved path; null ⇒ bundled default
}

export interface SessionRow {
  id: string;
  created_at: string;
  task: string | null;
  plan: string | null;
}

export interface MessageRow {
  role: string;
  content: string;
  ts: string;
}

export interface RunHandle {
  session_id: string;
}

export interface TaskOutput {
  session_id: string;
  stream: "stdout" | "stderr";
  line: string;
}

export interface TaskExit {
  session_id: string;
  code: number | null;
  success: boolean;
}

export interface BackgroundPayload {
  data_base64: string;
  content_type: string;
  is_default: boolean;
}

export interface BackgroundValidation {
  accepted: boolean;
  message: string;
  width: number;
  height: number;
  saved_path: string | null;
}

export interface HealthOutcome {
  provider_id: string;
  status: string;
  total_latency_ms: number;
  can_save: boolean;
  message: string;
  checks: HealthCheck[];
  models_discovered: string[];
}
export interface HealthCheck {
  label: string;
  passed: boolean;
  detail: string;
  latency_ms: number;
}

/**
 * Outcome of a per-role live API validation performed by the Model Gateway
 * (v0.15). Returned by `gateway_validate_role`. The result is persisted to
 * `~/.aether/validations.json` and gates Save/Activate in the UI.
 */
export interface RoleValidationDto {
  role: string;
  ok: boolean;
  /** Stable failure class (e.g. "rate_limited", "invalid_api_key"). */
  class: string | null;
  detail: string;
  latency_ms: number;
  /** Configuration fingerprint. Present only on success. */
  fingerprint: string | null;
  /** RFC3339-style timestamp of the successful validation. */
  validated_at: string | null;
}

/**
 * Validation status for Save/Activate gating. `valid` is true only when the
 * stored fingerprint matches the current config fingerprint.
 */
export interface RoleStatusDto {
  role: string;
  model_key: string;
  valid: boolean;
  reason: string | null;
  validated_at: string | null;
}

export interface SkillSummary {
  id: string;
  name: string;
  description: string;
  version: string;
  tags: string[];
  source_path: string;
}

export interface SnapshotRow {
  id: string;
  parent_id: string | null;
  timestamp: string;
  trigger: string;
  agent_id: string | null;
  task: string | null;
  files: string[];
  metadata: Record<string, string>;
}
export interface SnapshotResult {
  snapshot_id: string;
  files_restored: number;
  success: boolean;
  message: string;
}

// -- Code analysis (SonarQube capability, v0.14) -----------------------------

export interface AnalysisAvailability {
  available: boolean;
  detail: string;
}

export interface AnalysisFinding {
  id: string;
  rule: string;
  severity: string; // blocker | high | medium | low | info
  kind: string; // bug | vulnerability | security_hotspot | code_smell
  message: string;
  path: string;
  start_line: number;
  status: string;
  remediation: string | null;
}

export interface AnalysisReport {
  id: string;
  provider: string;
  project: string;
  at: string;
  label: string | null;
  finding_count: number;
  info: number;
  low: number;
  medium: number;
  high: number;
  blocker: number;
  affected_files: string[];
  findings: AnalysisFinding[];
}

export interface AnalysisRunResult {
  success: boolean;
  message: string;
  report: AnalysisReport | null;
}

export interface AnalysisDiff {
  resolved: string[];
  remaining: string[];
  introduced: string[];
  regressions: { fingerprint: string; old_severity: string; new_severity: string }[];
  baseline_count: number;
  current_count: number;
}

export interface AnalysisProgress {
  stage: string; // probing | analyzing | done | error
  findings?: number;
  message?: string;
}

export const api = {
  readConfig: () => invoke<ConfigResponse>("read_config"),
  writeConfig: (config: DesktopConfig) => invoke<string>("write_config", { config }),
  listSessions: () => invoke<SessionRow[]>("list_sessions"),
  getSessionMessages: (session_id: string) =>
    invoke<MessageRow[]>("get_session_messages", { sessionId: session_id }),
  runTask: (task: string, plan = false) =>
    invoke<RunHandle>("run_task", { task, plan }),
  cancelTask: (session_id: string) => invoke<boolean>("cancel_task", { sessionId: session_id }),
  aetherDir: () => invoke<string>("aether_dir_str"),
  version: () => invoke<string>("version"),
  backendStatus: () => invoke<{ mode: string; ready: boolean }>("backend_status"),
  getBackground: () => invoke<BackgroundPayload>("get_background"),
  setBackgroundImage: (bytes: number[]) =>
    invoke<BackgroundValidation>("set_background_image", { bytes }),
  requiredBackgroundResolution: () =>
    invoke<string>("required_background_resolution"),
  checkProvider: (baseUrl: string, apiKeyEnv: string, models: string[]) =>
    invoke<HealthOutcome>("check_provider", { baseUrl, apiKeyEnv, models }),
  /** Run a live API validation for one role and persist a fingerprint snapshot. */
  gatewayValidateRole: (role: string, config: DesktopConfig) =>
    invoke<RoleValidationDto>("gateway_validate_role", { role, config }),
  /** Look up current Save/Activate state for one role against the stored snapshot. */
  gatewayValidationStatus: (role: string, config: DesktopConfig) =>
    invoke<RoleStatusDto>("gateway_validation_status", { role, config }),
  listSkills: () => invoke<SkillSummary[]>("list_skills"),
  listSnapshots: (sessionId: string) =>
    invoke<SnapshotRow[]>("list_snapshots", { sessionId }),
  restoreSnapshot: (sessionId: string, snapshotId: string) =>
    invoke<SnapshotResult>("restore_snapshot", { sessionId, snapshotId }),
  snapshotUndo: (sessionId: string) =>
    invoke<SnapshotResult>("snapshot_undo", { sessionId }),
  snapshotRedo: (sessionId: string) =>
    invoke<SnapshotResult>("snapshot_redo", { sessionId }),
  // Code-analysis capability (v0.14)
  analysisCheck: (baseUrl?: string, tokenEnv?: string) =>
    invoke<AnalysisAvailability>("analysis_check", { baseUrl: baseUrl ?? null, tokenEnv: tokenEnv ?? null }),
  analysisRun: (
    projectRoot: string,
    opts?: { mode?: string; baseUrl?: string; tokenEnv?: string; scope?: string[]; label?: string },
  ) =>
    invoke<AnalysisRunResult>("analysis_run", {
      projectRoot,
      mode: opts?.mode ?? null,
      baseUrl: opts?.baseUrl ?? null,
      tokenEnv: opts?.tokenEnv ?? null,
      scope: opts?.scope ?? null,
      label: opts?.label ?? null,
    }),
  analysisLatest: (project: string) =>
    invoke<AnalysisReport | null>("analysis_latest", { project }),
  analysisProjects: () => invoke<string[]>("analysis_projects"),
  analysisDiff: (project: string, baselineReport: string, currentReport?: string) =>
    invoke<AnalysisDiff>("analysis_diff", {
      project,
      baselineReport,
      currentReport: currentReport ?? null,
    }),
};

export const events = {
  onTaskOutput: (cb: (e: TaskOutput) => void): Promise<UnlistenFn> =>
    listen<TaskOutput>("task-output", (e) => cb(e.payload)),
  onTaskExit: (cb: (e: TaskExit) => void): Promise<UnlistenFn> =>
    listen<TaskExit>("task-exit", (e) => cb(e.payload)),
  onAnalysisProgress: (cb: (e: AnalysisProgress) => void): Promise<UnlistenFn> =>
    listen<AnalysisProgress>("analysis-progress", (e) => cb(e.payload)),
};
