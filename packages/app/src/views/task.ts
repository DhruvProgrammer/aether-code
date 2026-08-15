import { api, events, type DesktopConfig, type TaskExit, type TaskOutput } from "../api";

const taskInput = document.querySelector<HTMLTextAreaElement>("#task-input")!;
const runBtn = document.querySelector<HTMLButtonElement>("#run-btn")!;
const cancelBtn = document.querySelector<HTMLButtonElement>("#cancel-btn")!;
const planChk = document.querySelector<HTMLInputElement>("#plan-chk")!;
const outputEl = document.querySelector<HTMLDivElement>("#task-output")!;
const statusEl = document.querySelector<HTMLSpanElement>("#task-status")!;

let currentSession: string | null = null;
let unlistenOut: (() => void) | null = null;
let unlistenExit: (() => void) | null = null;

export async function mountTask() {
  runBtn.addEventListener("click", start);
  cancelBtn.addEventListener("click", cancel);
  taskInput.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      start();
    }
  });

  unlistenOut = await events.onTaskOutput((o) => {
    if (o.session_id !== currentSession) return;
    appendLine(o.stream === "stderr" ? "stderr" : "stdout", o.line);
  });
  unlistenExit = await events.onTaskExit((e: TaskExit) => {
    if (e.session_id !== currentSession) return;
    setStatus(e.success ? "Done." : `Failed (exit ${e.code ?? "?"})`);
    currentSession = null;
    runBtn.disabled = false;
    cancelBtn.disabled = true;
  });
}

export function unmountTask() {
  unlistenOut?.();
  unlistenExit?.();
  unlistenOut = null;
  unlistenExit = null;
}

async function start() {
  const task = taskInput.value.trim();
  if (!task) return;
  outputEl.textContent = "";
  setStatus("Starting...");
  runBtn.disabled = true;
  cancelBtn.disabled = false;
  try {
    const handle = await api.runTask(task, planChk.checked);
    currentSession = handle.session_id;
    setStatus(`Running (${handle.session_id.slice(0, 12)}…)`);
  } catch (e) {
    setStatus(`Error: ${String(e)}`);
    runBtn.disabled = false;
    cancelBtn.disabled = true;
  }
}

async function cancel() {
  if (!currentSession) return;
  try {
    await api.cancelTask(currentSession);
    setStatus("Cancelling...");
  } catch (e) {
    setStatus(`Cancel failed: ${String(e)}`);
  }
}

function appendLine(stream: "stdout" | "stderr", line: string) {
  const span = document.createElement("span");
  span.className = stream === "stderr" ? "line stderr" : "line stdout";
  span.textContent = line + "\n";
  outputEl.appendChild(span);
  outputEl.scrollTop = outputEl.scrollHeight;
}

function setStatus(s: string) {
  statusEl.textContent = s;
}
