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
  locateCli: () => invoke<{ found: string | null; searched: string[] }>("locate_cli"),
  getBackground: () => invoke<BackgroundPayload>("get_background"),
  setBackgroundImage: (bytes: number[]) =>
    invoke<BackgroundValidation>("set_background_image", { bytes }),
  requiredBackgroundResolution: () =>
    invoke<string>("required_background_resolution"),
};

export const events = {
  onTaskOutput: (cb: (e: TaskOutput) => void): Promise<UnlistenFn> =>
    listen<TaskOutput>("task-output", (e) => cb(e.payload)),
  onTaskExit: (cb: (e: TaskExit) => void): Promise<UnlistenFn> =>
    listen<TaskExit>("task-exit", (e) => cb(e.payload)),
};
