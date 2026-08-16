// Thin wrapper over Tauri's invoke + event APIs.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ConfigResponse {
  path: string;
  exists: boolean;
  config: DesktopConfig;
}

export interface DesktopConfig {
  agent: {
    controller_model: string;
    executor_model: string;
    reviewer_model: string | null;
  };
  models: Record<string, ModelBlock>;
  frontend: {
    capture_command: string | null;
    preview_command: string | null;
    max_visual_iterations: number;
    force: boolean;
  };
}

export interface ModelBlock {
  provider: string;
  base_url: string;
  model: string;
  api_key_env: string;
  extra_body: unknown | null;
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
};

export const events = {
  onTaskOutput: (cb: (e: TaskOutput) => void): Promise<UnlistenFn> =>
    listen<TaskOutput>("task-output", (e) => cb(e.payload)),
  onTaskExit: (cb: (e: TaskExit) => void): Promise<UnlistenFn> =>
    listen<TaskExit>("task-exit", (e) => cb(e.payload)),
};
