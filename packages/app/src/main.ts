import {
  api,
  events,
  type TaskExit,
  type TaskOutput,
  type DesktopConfig,
  type AppearanceConfig,
  type WorkspaceDto,
  type SessionRowDto,
  type ProviderEntryDto,
  type RoleAssignmentsDto,
  type WorkspaceChangesDto,
} from "./api";

// ----- State -----

type BlockKind = "user" | "assistant" | "info" | "error" | "edit" | "tool" | "status" | "system";

interface Block {
  id: number;
  kind: BlockKind;
  text: string;
  meta?: string;
  pending?: boolean;
}

interface Session {
  id: string;
  title: string;
  blocks: Block[];
  running: boolean;
  nextId: number;
}

const app = {
  tabs: [] as Session[],
  active: 0,
  mode: "build" as "build" | "plan",
  // v0.17 workspace state
  workspace: null as WorkspaceDto | null,
  workspaceSessions: [] as SessionRowDto[],
  providers: [] as ProviderEntryDto[],
  roleAssignments: null as RoleAssignmentsDto | null,
  // realtime changes
  workspaceChanges: null as WorkspaceChangesDto | null,
  changesUnlisten: null as (() => void) | null,
  lastWatchedWorkspaceId: null as string | null,
};

function current(): Session {
  return app.tabs[app.active];
}

// ----- Helpers -----

const escapeHtml = (s: string): string =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

const escapeAttr = (s: string): string => escapeHtml(s);

function newId(s: Session): number {
  return s.nextId++;
}

function classifyLine(line: string): { kind: BlockKind; text: string } {
  const t = line.trim();
  if (t.startsWith("AETHER error") || t.startsWith("AETHER:")) return { kind: "error", text: line };
  if (t.startsWith("AETHER needs")) return { kind: "error", text: line };
  if (t.startsWith("[VISUAL]") || t.startsWith("[RESUME]") || t.startsWith("[background]")) return { kind: "status", text: line };
  if (t.startsWith("╭") || t.startsWith("│") || t.startsWith("╰") || t.startsWith("╮") || t.startsWith("╯")) return { kind: "system", text: line };
  if (t.startsWith("[") && t.includes("]")) return { kind: "info", text: line };
  if (t.length === 0) return { kind: "assistant", text: "" };
  return { kind: "assistant", text: line };
}

function validateBeforeSend(): string | null {
  if (!app.workspace) return "No workspace selected — select a folder first.";
  if (app.providers.length === 0) return "No providers configured. Open Settings → LLM Providers and add a provider first. [/settings]";
  if (!app.roleAssignments?.executor || !app.roleAssignments?.controller) {
    return "No model configured for LLM 1 (Executor) or LLM 2 (Controller). Click the model selector below (— not set —) and assign a provider/model for each role before sending. LLM 3 is optional.";
  }
  const exec = app.roleAssignments.executor!;
  const ctrl = app.roleAssignments.controller!;
  const execProv = app.providers.find((p) => p.id === exec.provider_id);
  if (!execProv) return `Executor provider '${exec.provider_id}' not found. Reconfigure LLM 1 in the model selector.`;
  if (!execProv.models.some((m) => m.id === exec.model_id)) return `Executor model '${exec.model_id}' not found in provider '${exec.provider_id}'. Reconfigure LLM 1 or add the model in Settings.`;
  const ctrlProv = app.providers.find((p) => p.id === ctrl.provider_id);
  if (!ctrlProv) return `Controller provider '${ctrl.provider_id}' not found. Reconfigure LLM 2.`;
  if (!ctrlProv.models.some((m) => m.id === ctrl.model_id)) return `Controller model '${ctrl.model_id}' not found in provider '${ctrl.provider_id}'.`;
  if (!execProv.base_url || !execProv.base_url.trim()) return `Provider '${execProv.id}' has no Base URL configured. Open Settings and set it.`;
  if (!execProv.api_key_env || !execProv.api_key_env.trim()) return `Provider '${execProv.id}' has no API Key configured. Open Settings and set it.`;
  if (!ctrlProv.base_url || !ctrlProv.base_url.trim()) return `Provider '${ctrlProv.id}' has no Base URL configured.`;
  if (app.roleAssignments.reviewer) {
    const rev = app.roleAssignments.reviewer;
    const revProv = app.providers.find((p) => p.id === rev.provider_id);
    if (!revProv) return `Reviewer provider '${rev.provider_id}' not found.`;
    if (!revProv.models.some((m) => m.id === rev.model_id)) return `Reviewer model '${rev.model_id}' not found in provider '${rev.provider_id}'.`;
  }
  return null;
}

function appendLine(s: Session, line: string) {
  const cls = classifyLine(line);
  const last = s.blocks[s.blocks.length - 1];
  if (cls.kind === "assistant" && last && last.kind === "assistant") {
    last.text += (last.text ? "\n" : "") + cls.text;
  } else if (cls.kind === "info" && last && last.kind === "info") {
    last.text += "\n" + cls.text;
  } else if (cls.kind === "system" && last && last.kind === "system") {
    last.text += "\n" + cls.text;
  } else {
    s.blocks.push({ id: newId(s), kind: cls.kind, text: cls.text });
  }
}

// ----- Rendering -----

function renderTabs() {
  const el = document.querySelector<HTMLDivElement>("#tabs")!;
  el.innerHTML = app.tabs
    .map((s, i) => {
      const cls = i === app.active ? "bg-app-surface border-app-border" : "border-transparent hover:bg-app-hover";
      const labelCls = i === app.active ? "text-app-brand" : "text-app-textSecondary";
      return `<div class="flex items-center ${i === app.active ? "bg-app-surface" : ""} rounded-md px-3 py-1 border ${cls} cursor-pointer group relative" data-tab="${i}">
        <div class="bg-app-brand text-app-bg text-xs font-bold px-1.5 py-0.5 rounded mr-2">T</div>
        <span class="text-sm font-medium pr-6 truncate max-w-[200px] ${labelCls}">${escapeHtml(s.title)}</span>
        <button type="button" class="absolute right-2 text-app-textSecondary hover:text-app-error" data-close-tab="${i}" title="Close tab">
          <i class="w-3.5 h-3.5" data-lucide="x"></i>
        </button>
      </div>`;
    })
    .join("");
}

function renderStream() {
  const s = current();
  const container = document.querySelector<HTMLDivElement>("#messages")!;
  if (s.blocks.length === 0) {
    container.innerHTML = `<div class="text-app-textSecondary text-sm italic" id="empty-hint">
      Ask aether anything. <kbd class="bg-app-surface px-1.5 py-0.5 rounded border border-app-border font-mono text-xs">/</kbd> for commands,
      <kbd class="bg-app-surface px-1.5 py-0.5 rounded border border-app-border font-mono text-xs">@</kbd> for context.
    </div>`;
  } else {
    container.innerHTML = s.blocks.map(renderBlock).join("");
  }
  const stream = document.querySelector<HTMLDivElement>("#stream")!;
  stream.scrollTop = stream.scrollHeight;
}

function renderBlock(b: Block): string {
  switch (b.kind) {
    case "user":
      return `<div class="msg-block flex justify-end">
        <div class="bg-app-brand text-app-bg rounded-2xl px-4 py-2 max-w-[80%] text-sm shadow">
          ${escapeHtml(b.text)}
        </div>
      </div>`;
    case "assistant":
      return `<div class="msg-block flex items-start space-x-3">
        <div class="w-8 h-8 rounded-lg bg-app-brand text-app-bg flex items-center justify-center text-xs font-bold shrink-0">AE</div>
        <div class="text-app-textPrimary leading-relaxed whitespace-pre-wrap text-sm flex-1 pt-1">${escapeHtml(b.text)}</div>
      </div>`;
    case "info":
      return `<div class="msg-block pl-4 border-l-2 border-app-brand py-1 text-app-textSecondary text-xs font-mono whitespace-pre-wrap">${escapeHtml(b.text)}</div>`;
    case "status":
      return `<div class="msg-block pl-4 border-l-2 border-app-brand py-1 text-app-textSecondary text-xs font-mono">${escapeHtml(b.text)}</div>`;
    case "system":
      return `<div class="msg-block text-app-textSecondary text-xs font-mono whitespace-pre-wrap pl-2">${escapeHtml(b.text)}</div>`;
    case "error":
      return `<div class="msg-block pl-4 border-l-2 border-app-error py-1 text-app-error text-sm">${escapeHtml(b.text)}</div>`;
    case "edit":
      return `<div class="msg-block pl-4 border-l-2 border-app-warning py-1">
        <div class="text-app-textSecondary text-xs uppercase tracking-wide mb-1">Edit</div>
        <pre class="text-app-textPrimary font-mono text-xs whitespace-pre-wrap">${escapeHtml(b.text)}</pre>
      </div>`;
    case "tool":
      return `<div class="msg-block pl-4 border-l-2 border-app-brand py-1">
        <div class="text-app-textSecondary text-xs uppercase tracking-wide mb-1">Tool</div>
        <pre class="text-app-textPrimary font-mono text-xs whitespace-pre-wrap">${escapeHtml(b.text)}</pre>
      </div>`;
  }
}

function renderHeader() {
  const s = current();
  document.querySelector("#session-title")!.textContent = s.title;
  const pill = document.querySelector<HTMLDivElement>("#status-pill")!;
  if (s.running) {
    pill.classList.remove("hidden");
    pill.classList.add("flex");
    document.querySelector("#status-text")!.textContent = "Working…";
  } else {
    pill.classList.add("hidden");
    pill.classList.remove("flex");
  }
  document.querySelector("#mode-label")!.textContent = app.mode === "build" ? "Build" : "Plan";
  document.querySelector("#model-label")!.textContent = bindingLabel(app.roleAssignments?.executor ?? null);
}

function renderAll() {
  renderTabs();
  renderHeader();
  renderStream();
  // Re-render lucide icons after any innerHTML swap.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const lucide = (window as any).lucide;
  if (lucide?.createIcons) lucide.createIcons();
}

// ----- Sessions -----

function createSession(title?: string): Session {
  return {
    id: crypto.randomUUID(),
    title: title ?? "New chat",
    blocks: [],
    running: false,
    nextId: 1,
  };
}

function newTab() {
  const s = createSession();
  app.tabs.push(s);
  app.active = app.tabs.length - 1;
  renderAll();
}

// ----- Run / send -----

// Per-session listener registry — one tab's task must never detach another's handlers.
const sessionUnlistens = new Map<string, (() => void)[]>();

function redactSecrets(msg: string): string {
  return msg
    .replace(/\bsk-[a-zA-Z0-9_\-]{8,}/g, "[REDACTED]")
    .replace(/\bnvapi-[a-zA-Z0-9_\-]{8,}/g, "[REDACTED]")
    .replace(/\bvenice-[a-zA-Z0-9_\-]{8,}/g, "[REDACTED]")
    .replace(/\bBearer\s+[a-zA-Z0-9_\-\.]{12,}/g, "Bearer [REDACTED]")
    .replace(/\b(api[_-]?key|authorization|x-api-key)\s*[:=]\s*\S+/gi, "$1: [REDACTED]");
}

async function send() {
  const input = document.querySelector<HTMLTextAreaElement>("#prompt-input")!;
  const text = input.value.trim();
  if (!text) return;

  const s = current();

  // Running guard: never double-fire a task in the same session.
  if (s.running) {
    s.blocks.push({ id: newId(s), kind: "info", text: "A task is already running in this tab. Use /cancel to stop it first." });
    renderAll();
    return;
  }

  // NO_WORKSPACE guard (v0.17): coding tasks require an active workspace.
  if (!app.workspace) {
    s.blocks.push({
      id: newId(s),
      kind: "info",
      text: "Please select a folder to start working.",
    });
    renderAll();
    void pickFolder();
    return; // input preserved so user doesn't lose their message
  }

  // Validate BEFORE consuming the user's message (fixes lost-input papercut).
  const precheck = validateBeforeSend();
  if (precheck) {
    s.blocks.push({ id: newId(s), kind: "error", text: precheck });
    renderAll();
    return; // input preserved
  }

  // Only now consume the message.
  if (!text.startsWith("/")) {
    s.title = text.slice(0, 40);
  }
  s.blocks.push({ id: newId(s), kind: "user", text });
  input.value = "";
  input.style.height = "auto";

  if (text.startsWith("/help")) {
    s.blocks.push({
      id: newId(s),
      kind: "info",
      text: [
        "/help         Show this help",
        "/plan         Switch to plan (read-only) mode",
        "/build        Switch to build mode",
        "/clear        Clear this session's messages",
        "/settings     Open the Settings modal",
        "/history      Open the History modal",
        "/cancel       Cancel the running task",
        "/compact      Compact session context (structured checkpoint)",
        "/new          Start a new tab",
        "/status       Show internal backend status",
      ].join("\n"),
    });
    renderAll();
    return;
  }

  if (text === "/clear") { s.blocks = []; renderAll(); return; }
  if (text === "/plan") { app.mode = "plan"; renderAll(); return; }
  if (text === "/build") { app.mode = "build"; renderAll(); return; }
  if (text === "/new") { newTab(); return; }
  if (text === "/settings") { openModal("settings"); return; }
  if (text === "/history") { openModal("history"); return; }
  if (text === "/compact") {
    s.blocks.push({ id: newId(s), kind: "status", text: "Compacting context…" });
    renderAll();
    try {
      const r = await api.compactSession(s.id);
      s.blocks.push({
        id: newId(s),
        kind: r.status === "completed" ? "info" : "error",
        text: `${r.message} (${r.tokens_before} → ${r.tokens_after} tokens)`,
      });
    } catch (e) {
      s.blocks.push({ id: newId(s), kind: "error", text: `Compact failed: ${redactSecrets(String(e))}` });
    }
    renderAll();
    return;
  }
  if (text === "/cancel") {
    try {
      const ok = await api.cancelTask(s.id);
      s.blocks.push({ id: newId(s), kind: "info", text: ok ? "Cancel requested — stopping at the next safe boundary…" : "No running task found for this session." });
    } catch (e) {
      s.blocks.push({ id: newId(s), kind: "error", text: `Cancel failed: ${redactSecrets(String(e))}` });
    }
    renderAll();
    return;
  }
  if (text === "/status") {
    try {
      const r = await api.backendStatus();
      s.blocks.push({ id: newId(s), kind: "info", text: `Backend: ${r.backend}\nConfig: ${r.config_path} (${r.config_exists ? "found" : "missing"})\nVersion: v${r.version}` });
    } catch (e) {
      s.blocks.push({ id: newId(s), kind: "error", text: `status failed: ${redactSecrets(String(e))}` });
    }
    renderAll();
    return;
  }

  // Real task.
  s.running = true;
  renderAll();
  attachListeners(s);

  // Send button disabled state while running.
  const sendBtn = document.querySelector<HTMLButtonElement>("#send-btn");
  sendBtn?.setAttribute("disabled", "true");

  try {
    const handle = await api.runTask(text, app.mode === "plan", {
      sessionId: s.id,
      workspacePath: app.workspace?.path,
      roleAssignmentsJson: app.roleAssignments ? JSON.stringify(app.roleAssignments) : undefined,
    });
    s.id = handle.session_id;
    // Re-key listeners under the backend-assigned session id.
    rekeyListeners(s);
  } catch (e) {
    s.running = false;
    s.blocks.push({ id: newId(s), kind: "error", text: `Failed to start: ${redactSecrets(String(e))}` });
    renderAll();
    detachListeners(s.id);
    sendBtn?.removeAttribute("disabled");
  }
}

function rekeyListeners(s: Session): void {
  const fns = sessionUnlistens.get("__pending__");
  if (fns) {
    sessionUnlistens.delete("__pending__");
    sessionUnlistens.set(s.id, fns);
  }
}

function attachListeners(s: Session) {
  detachListeners(s.id);
  const unlistens: (() => void)[] = [];
  sessionUnlistens.set("__pending__", unlistens);
  let sid = s.id;
  events.onTaskOutput((o: TaskOutput) => {
    try {
      if (o.session_id !== sid && o.session_id !== "__pending__") return;
      if (o.stream === "stderr") {
        // Provider/gateway errors are human-readable; redact any leaked secrets.
        s.blocks.push({ id: newId(s), kind: "error", text: redactSecrets(o.line || "") });
      } else {
        appendLine(s, o.line);
      }
      // Only re-render when this session is the visible tab (fixes scroll-yank).
      if (current() === s) renderStream();
    } catch (e) {
      console.error("task-output handler failed", e);
      try { s.blocks.push({ id: newId(s), kind: "error", text: `Rendering error: ${String(e)}` }); if (current() === s) renderStream(); } catch {}
    }
  }).then((u) => unlistens.push(u));
  events.onTaskExit((e: TaskExit) => {
    try {
      if (e.session_id !== sid) return;
      s.running = false;
      if (!e.success) {
        const code = e.code ?? "?";
        if (!s.blocks.some((b) => b.kind === "error" && b.text.includes("exit code"))) {
          s.blocks.push({ id: newId(s), kind: "error", text: `Task finished with exit code: ${code}. Check provider errors above.` });
        }
      }
      if (current() === s) renderAll();
    } catch (err) {
      console.error("task-exit handler failed", err);
    } finally {
      detachListeners(e.session_id);
      document.querySelector<HTMLButtonElement>("#send-btn")?.removeAttribute("disabled");
    }
  }).then((u) => unlistens.push(u));
  events.onTaskState?.((st) => {
    try {
      if (st.session_id !== sid) return;
      console.debug("task-state", st.payload);
    } catch {}
  }).then((u) => { if (u) unlistens.push(u); }).catch(()=>{});
  // After runTask resolves with a real session id, re-key under it.
  const origId = sid;
  void Promise.resolve().then(() => { if (s.id !== origId) { sid = s.id; rekeyListeners(s); } });
}

function detachListeners(sessionId?: string) {
  if (sessionId === undefined) {
    for (const fns of sessionUnlistens.values()) for (const u of fns) try { u(); } catch {}
    sessionUnlistens.clear();
    return;
  }
  const fns = sessionUnlistens.get(sessionId) ?? sessionUnlistens.get("__pending__");
  if (fns) {
    for (const u of fns) try { u(); } catch {}
    sessionUnlistens.delete(sessionId);
    sessionUnlistens.delete("__pending__");
  }
}

// ----- Modal (settings + history) -----

async function openModal(kind: "settings" | "history") {
  const modal = document.querySelector<HTMLDivElement>("#modal")!;
  const title = document.querySelector("#modal-title")!;
  const body = document.querySelector<HTMLDivElement>("#modal-body")!;
  title.textContent = kind === "settings" ? "Settings" : "History";
  modal.classList.remove("hidden");
  modal.classList.add("flex");

  if (kind === "settings") {
    body.innerHTML = `<div class="text-app-textSecondary text-sm">Loading…</div>`;
    try {
      const r = await api.readConfig();
      body.innerHTML = await renderSettings(r.config, r.path);
      wireSettings(body, r.config);
      await refreshBackgroundPreview(body);
      applyBackgroundFromSettings(body);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const lucide = (window as any).lucide;
      if (lucide?.createIcons) lucide.createIcons();
    } catch (e) {
      body.innerHTML = `<div class="text-app-error text-sm">Failed: ${escapeHtml(String(e))}</div>`;
    }
  } else {
    body.innerHTML = `<div class="text-app-textSecondary text-sm">Loading…</div>`;
    try {
      const rows = await api.listSessions();
      body.innerHTML = renderHistory(rows);
      wireHistory(body);
    } catch (e) {
      body.innerHTML = `<div class="text-app-error text-sm">Failed: ${escapeHtml(String(e))}</div>`;
    }
  }
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const lucide = (window as any).lucide;
  if (lucide?.createIcons) lucide.createIcons();
}

function closeModal() {
  const modal = document.querySelector<HTMLDivElement>("#modal")!;
  modal.classList.add("hidden");
  modal.classList.remove("flex");
}

// Dirty-check guard: warn before discarding typed form data in provider/model modals.
let providerModalDirty = false;
let modelModalDirty = false;

function confirmDiscard(kind: "provider" | "model"): boolean {
  const dirty = kind === "provider" ? providerModalDirty : modelModalDirty;
  if (!dirty) return true;
  return window.confirm(`You have unsaved changes in this ${kind} form. Discard them?`);
}

// ---------------------------------------------------------------------------
// Settings (spec §1-§23)
// ---------------------------------------------------------------------------
//
// The settings UI is organised in three horizontal sections:
//
//   AI / MODELS     — three compact cards side-by-side: Model 1 (Big Executor),
//                     Model 2 (Small Controller), Model 3 (Visual Reviewer).
//                     Each card exposes Model ID, Provider, Base URL, API Key.
//                     Slots 2 and 3 may be left blank to disable them.
//
//   PROVIDERS       — OpenAI-compatible provider entries. Per spec §8 each
//                     entry contains ONLY: Provider ID, Base URL, API Key,
//                     Models. Display Name and Headers are intentionally
//                     absent.
//
//   APPEARANCE      — native AETHER background image (enabled toggle, live
//                     preview, opacity slider, display mode). Any valid image
//                     is accepted; the renderer adapts to the viewport via
//                     CSS object-fit (fill/fit/stretch/center).
//
// Validation rules (spec §7):
//   * Model 1 — every field required.
//   * Model 2/3 — completely empty is VALID; partially configured is INVALID
//     and the UI shows which field is missing.

const providerValidation = new Map<number, { checking: boolean; ok?: boolean; detail: string }>();
const modelValidation = new Map<string, { checking: boolean; ok?: boolean; detail: string }>();

function maskApiKey(s: string): string {
  if (!s) return "—";
  if (s.length <= 4) return "••••";
  return "•".repeat(Math.min(s.length, 16));
}

function renderProviderCatalog(): string {
  if (app.providers.length === 0) {
    return `<div class="text-xs text-app-textSecondary italic py-4 text-center">No providers configured yet. Add one below.</div>`;
  }
  return app.providers.map((p, idx) => {
    const pv = providerValidation.get(idx);
    const pvLabel = pv ? (pv.checking ? "Checking…" : pv.ok ? "✓ Connected" : `✗ ${escapeHtml(pv.detail.slice(0,80))}`) : "— not checked —";
    const pvClass = pv?.checking ? "text-app-textSecondary" : pv?.ok ? "text-green-400" : pv?.ok === false ? "text-app-error" : "text-app-textSecondary";
    const headers = p.headers && typeof p.headers === "object" && !Array.isArray(p.headers) ? Object.entries(p.headers as Record<string,string>) : [];
    return `
    <div class="provider-card" data-provider-card="${idx}">
      <div class="provider-header-row">
        <div>
          <div class="slot-title">${escapeHtml(p.display_name || p.id)} <span class="text-[10px] font-mono text-app-textSecondary">(${escapeHtml(p.id)})</span></div>
          <div class="text-xs text-app-textSecondary">${escapeHtml(p.protocol)}</div>
        </div>
        <div class="flex items-center space-x-1">
          <button type="button" data-provider-edit="${idx}" class="text-xs px-2 py-1 border border-app-border rounded text-app-textSecondary hover:text-app-brand hover:border-app-brand">Edit</button>
          <button type="button" data-provider-del="${idx}" class="text-app-textSecondary hover:text-app-error p-1" title="Delete provider"><i class="w-4 h-4" data-lucide="trash-2"></i></button>
        </div>
      </div>
      <div class="mt-3 space-y-2 text-xs">
        <div><span class="text-app-textSecondary">Base URL:</span> <code class="font-mono text-app-textPrimary">${escapeHtml(p.base_url || "— not set —")}</code></div>
        <div><span class="text-app-textSecondary">API Key:</span> <code class="font-mono">${p.auth_type === "raw" ? maskApiKey(p.api_key ?? p.api_key_env ?? "") : p.auth_type === "none" ? "— none —" : escapeHtml(p.api_key_env || "— not set —")}</code> <span class="text-[10px] text-app-textSecondary">(${escapeHtml(p.auth_type === "raw" ? "raw" : p.auth_type === "none" ? "none" : p.api_key_env ? p.auth_type ?? "env_var" : "empty")})</span></div>
        ${headers.length ? `<div><span class="text-app-textSecondary">Headers:</span> <span class="font-mono text-[11px]">${headers.map(([k,v])=> escapeHtml(k)+": "+maskApiKey(String(v))).join(", ")}</span></div>` : ""}
      </div>
      <div class="flex items-center space-x-2 mt-3">
        <button type="button" data-provider-check="${idx}" class="text-xs px-3 py-1.5 border border-app-border rounded hover:border-app-brand text-app-textSecondary hover:text-app-textPrimary">${pv?.checking ? "Checking…" : "Check Connection"}</button>
        <span data-provider-status="${idx}" class="text-xs font-mono ${pvClass}">${pvLabel}</span>
      </div>
      <div class="mt-4">
        <div class="text-xs uppercase tracking-wide text-app-textSecondary mb-2">Models (${p.models.length})</div>
        <div class="space-y-1">
          ${p.models.length ? p.models.map((m, mid) => {
            const key = `${idx}:${mid}`;
            const mv = modelValidation.get(key);
            const mvLabel = mv ? (mv.checking ? "Checking…" : mv.ok ? "✓ OK" : `✗ ${escapeHtml(mv.detail.slice(0,60))}`) : "—";
            const mvClass = mv?.checking ? "text-app-textSecondary" : mv?.ok ? "text-green-400" : mv?.ok===false ? "text-app-error" : "text-app-textSecondary";
            return `<div class="flex items-center justify-between text-xs px-3 py-2 bg-app-bg rounded border border-app-border/50">
              <div class="min-w-0">
                <div class="font-mono text-app-textPrimary truncate">${escapeHtml(m.display_name || m.id)} <span class="text-[10px] text-app-textSecondary">(${escapeHtml(m.id)})</span></div>
                <div class="text-[10px] text-app-textSecondary">${m.tool_calling?"tools ":""}${m.vision?"vision ":""}${m.streaming?"streaming":""} ${m.context_window?`· ${m.context_window} ctx`: ""}</div>
                <div class="text-[10px] font-mono ${mvClass}">${mvLabel}</div>
              </div>
              <div class="flex items-center space-x-1 shrink-0 ml-2">
                <button type="button" data-model-check="${idx}|${mid}" class="text-[10px] px-2 py-1 border border-app-border rounded hover:border-app-brand text-app-textSecondary">${mv?.checking?"…":"Check"}</button>
                <button type="button" data-model-edit="${idx}|${mid}" class="text-app-textSecondary hover:text-app-brand p-1" title="Edit model"><i class="w-3.5 h-3.5" data-lucide="pencil"></i></button>
                <button type="button" data-model-del="${idx}|${mid}" class="text-app-textSecondary hover:text-app-error p-1" title="Remove model"><i class="w-3.5 h-3.5" data-lucide="trash-2"></i></button>
              </div>
            </div>`;
          }).join("") : `<div class="text-xs text-app-textSecondary italic py-2">No models — add one. Credentials are stored once per provider.</div>`}
        </div>
        <button type="button" data-add-model="${idx}" class="mt-2 text-xs text-app-brand hover:underline inline-flex items-center"><i class="w-3 h-3 mr-1" data-lucide="plus"></i> Add Model</button>
      </div>
    </div>`;
  }).join("");
}

async function renderSettings(cfg: DesktopConfig, path: string): Promise<string> {
  const appearance: AppearanceConfig = cfg.appearance ?? {
    background_enabled: true,
    background_opacity: 60,
    background_image: null,
    background_mode: "fill",
  };
  const bgMode = appearance.background_mode ?? "fill";

  return `
    <div class="space-y-1">
      <p class="text-app-textSecondary text-xs">Saved to <code class="font-mono">${escapeHtml(path)}</code></p>

      <!-- ───── LLM PROVIDERS (v0.17 catalog) ───── -->
      <section class="settings-section">
        <h3>LLM Providers</h3>
        <p class="text-xs text-app-textSecondary mb-3">Add any number of providers. Each provider can have multiple models. Credentials are stored once per provider.</p>
        <div id="provider-catalog" class="space-y-3">
          ${renderProviderCatalog()}
        </div>
        <button id="add-provider-v2" type="button" class="mt-3 text-xs text-app-textSecondary hover:text-app-brand inline-flex items-center">
          <i class="w-3.5 h-3.5 mr-1" data-lucide="plus"></i> Add Provider
        </button>
      </section>

      <!-- ───── APPEARANCE ───── -->
      <section class="settings-section">
        <h3>Appearance</h3>
        <p class="text-xs text-app-textSecondary mb-3">Upload an image to use as your AETHER background. Any valid image works; it adapts to your window size.</p>

        <div class="flex items-center mb-4">
          <div id="bg-toggle" class="toggle ${appearance.background_enabled ? "on" : ""}">
            <div class="knob"></div>
            <span class="label">Background Image ${appearance.background_enabled ? "ON" : "OFF"}</span>
          </div>
        </div>

        <div class="bg-preview mb-3">
          <img id="bg-preview-img" alt="background preview" />
        </div>

        <div class="flex items-center space-x-4">
          <label class="text-xs uppercase tracking-wide text-app-textSecondary w-20">Opacity</label>
          <input id="bg-opacity" type="range" min="0" max="100" value="${appearance.background_opacity}" class="opacity-slider flex-1" />
          <span id="bg-opacity-label" class="text-xs font-mono w-12 text-right">${appearance.background_opacity}%</span>
        </div>

        <div class="flex items-center space-x-4 mt-3">
          <label class="text-xs uppercase tracking-wide text-app-textSecondary w-20">Display</label>
          <select id="bg-mode" class="bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono text-app-textPrimary">
            <option value="fill" ${bgMode === "fill" ? "selected" : ""}>Fill (cover)</option>
            <option value="fit" ${bgMode === "fit" ? "selected" : ""}>Fit (contain)</option>
            <option value="stretch" ${bgMode === "stretch" ? "selected" : ""}>Stretch</option>
            <option value="center" ${bgMode === "center" ? "selected" : ""}>Center</option>
          </select>
        </div>

        <div class="mt-3 flex items-center space-x-3">
          <button id="bg-upload" type="button" class="text-xs px-3 py-1.5 border border-app-border rounded hover:border-app-brand text-app-textSecondary hover:text-app-textPrimary inline-flex items-center">
            <i class="w-3.5 h-3.5 mr-1.5" data-lucide="upload"></i> Upload image
          </button>
          <input id="bg-upload-input" type="file" accept="image/png,image/jpeg" style="display:none" />
          <button id="bg-reset" type="button" class="text-xs px-3 py-1.5 border border-app-border rounded hover:border-app-brand text-app-textSecondary hover:text-app-textPrimary inline-flex items-center">
            <i class="w-3.5 h-3.5 mr-1.5" data-lucide="rotate-ccw"></i> Reset to default
          </button>
        </div>
        <div class="validation-err" id="bg-validation" style="display:none"></div>
      </section>

      <!-- ───── SUBSYSTEMS (v0.12) ───── -->
      <section class="settings-section">
        <h3>Subsystems</h3>
        <p class="text-xs text-app-textSecondary mb-3">Five first-class systems ship with the runtime. Each is independently auditable.</p>
        <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
          <div class="model-card">
            <div class="slot-title">CONTEXT MANAGER</div>
            <div class="role-desc">Automatic compaction + per-agent segmentation</div>
            <div class="validation-ok mt-2">Active — segments: 17 kinds · thresholds: 70/82/94%</div>
          </div>
          <div class="model-card">
            <div class="slot-title">PERMISSION ENGINE</div>
            <div class="role-desc">Hierarchical: Global → Project → Role → Agent → Tool</div>
            <div class="validation-ok mt-2">Active — decision log + approval channel wired</div>
          </div>
          <div class="model-card">
            <div class="slot-title">SKILL REGISTRY</div>
            <div class="role-desc">On-demand capability packages</div>
            <div id="skill-list" class="text-xs text-app-textSecondary mt-2">Loading…</div>
          </div>
          <div class="model-card">
            <div class="slot-title">PROVIDER / MODEL CATALOG</div>
            <div class="role-desc">Health-check before activation</div>
            <div class="mt-2 flex flex-col space-y-2">
              <input id="hc-url" class="bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" placeholder="https://api.example.com/v1" />
              <input id="hc-env" class="bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" placeholder="API_KEY_ENV" />
              <input id="hc-model" class="bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" placeholder="model-id" />
              <button id="hc-run" type="button" class="text-xs px-3 py-1.5 border border-app-border rounded hover:border-app-brand text-app-textSecondary hover:text-app-textPrimary">Check Provider</button>
              <div id="hc-out" class="text-xs font-mono whitespace-pre-wrap text-app-textSecondary"></div>
            </div>
          </div>
          <div class="model-card col-span-full">
            <div class="slot-title">SNAPSHOTS / UNDO / REDO</div>
            <div class="role-desc">State-aware recovery · Undo understands multi-agent changes</div>
            <div class="mt-2 flex items-center space-x-2">
              <input id="snap-session" class="flex-1 bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" placeholder="session-id" />
              <button id="snap-list" type="button" class="text-xs px-3 py-1.5 border border-app-border rounded hover:border-app-brand text-app-textSecondary hover:text-app-textPrimary">List</button>
              <button id="snap-undo" type="button" class="text-xs px-3 py-1.5 border border-app-border rounded hover:border-app-brand text-app-textSecondary hover:text-app-textPrimary">Undo</button>
              <button id="snap-redo" type="button" class="text-xs px-3 py-1.5 border border-app-border rounded hover:border-app-brand text-app-textSecondary hover:text-app-textPrimary">Redo</button>
            </div>
            <div id="snap-list-out" class="text-xs font-mono mt-2 max-h-40 overflow-y-auto"></div>
          </div>
          <div class="model-card col-span-full">
            <div class="slot-title">CODE ANALYSIS — SONARQUBE</div>
            <div class="role-desc">Deterministic static analysis capability. Findings inform the controller; the controller decides fixes. Tokens stay in environment variables.</div>
            <div class="mt-2 grid grid-cols-1 md:grid-cols-3 gap-2">
              <input id="sa-root" class="md:col-span-2 bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" placeholder="Project root (absolute path)" />
              <div class="flex items-center space-x-2">
                <button id="sa-run" type="button" class="text-xs px-3 py-1.5 border border-app-border rounded hover:border-app-brand text-app-textSecondary hover:text-app-textPrimary inline-flex items-center"><i class="w-3.5 h-3.5 mr-1" data-lucide="scan"></i> Run</button>
                <button id="sa-mode" type="button" class="text-xs px-2 py-1.5 border border-app-border rounded hover:border-app-brand text-app-textSecondary hover:text-app-textPrimary" title="Toggle scan mode: run (fetch current results) / scanner (launch sonar-scanner first)">run</button>
              </div>
            </div>
            <div class="mt-2 grid grid-cols-1 md:grid-cols-2 gap-2">
              <input id="sa-url" class="bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" placeholder="SonarQube URL (default: SONAR_HOST_URL / http://localhost:9000)" />
              <input id="sa-token-env" class="bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" placeholder="Token env var name (default: SONAR_TOKEN)" />
            </div>
            <div id="sa-status" class="text-xs font-mono mt-2 text-app-textSecondary whitespace-pre-wrap"></div>
            <div id="sa-report" class="mt-2"></div>
          </div>
        </div>
      </section>

      <!-- ───── Save bar ───── -->
      <div class="flex items-center space-x-3 pt-4 border-t border-app-border">
        <button id="save-btn" type="button" class="bg-app-brand text-app-bg font-semibold px-4 py-1.5 rounded text-sm hover:opacity-90 inline-flex items-center">
          <i class="w-3.5 h-3.5 mr-1.5" data-lucide="save"></i> Save
        </button>
        <span id="save-status" class="text-xs text-app-textSecondary"></span>
      </div>
    </div>`;
}

function collectAppearance(body: HTMLElement, current: AppearanceConfig): AppearanceConfig {
  const enabled = body.querySelector<HTMLDivElement>("#bg-toggle")?.classList.contains("on") ?? current.background_enabled;
  const opacity = parseInt(body.querySelector<HTMLInputElement>("#bg-opacity")?.value ?? String(current.background_opacity), 10);
  const mode = body.querySelector<HTMLSelectElement>("#bg-mode")?.value ?? current.background_mode ?? "fill";
  return {
    background_enabled: enabled,
    background_opacity: isNaN(opacity) ? current.background_opacity : Math.max(0, Math.min(100, opacity)),
    background_image: current.background_image,
    background_mode: mode,
  };
}

function refreshProviderCatalog(body: HTMLElement): void {
  const catalog = body.querySelector<HTMLDivElement>("#provider-catalog");
  if (!catalog) return;
  catalog.innerHTML = renderProviderCatalog();
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const lucide = (window as any).lucide;
  if (lucide?.createIcons) lucide.createIcons();
  wireProviderCatalog(body);
}

function isValidProviderId(id: string): boolean {
  return /^[a-z0-9_-]+$/.test(id) && id.length >= 2 && id.length <= 40;
}

function wireProviderCatalog(body: HTMLElement): void {
  body.querySelector<HTMLButtonElement>("#add-provider-v2")?.addEventListener("click", () => openProviderModal(null));

  body.querySelectorAll<HTMLButtonElement>("[data-provider-edit]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const idx = parseInt(btn.dataset.providerEdit!, 10);
      openProviderModal(idx);
    });
  });

  body.querySelectorAll<HTMLButtonElement>("[data-provider-del]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const idx = parseInt(btn.dataset.providerDel!, 10);
      app.providers.splice(idx, 1);
      providerValidation.delete(idx);
      // reindex validation maps
      const newMap = new Map<number, { checking: boolean; ok?: boolean; detail: string }>();
      providerValidation.forEach((v, k) => {
        if (k < idx) newMap.set(k, v);
        else if (k > idx) newMap.set(k - 1, v);
      });
      providerValidation.clear();
      newMap.forEach((v, k) => providerValidation.set(k, v));
      refreshProviderCatalog(body);
    });
  });

  body.querySelectorAll<HTMLButtonElement>("[data-provider-check]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const idx = parseInt(btn.dataset.providerCheck!, 10);
      const prov = app.providers[idx];
      providerValidation.set(idx, { checking: true, detail: "checking…" });
      refreshProviderCatalog(body);
      try {
        const out = await api.providerCheckConnection(prov.id);
        const ok = out.can_save;
        providerValidation.set(idx, { checking: false, ok, detail: out.message + (out.checks?.length ? " — " + out.checks.map(c=> `${c.label}:${c.passed?"ok":"fail"}`).join(", ") : "") });
      } catch (e) {
        providerValidation.set(idx, { checking: false, ok: false, detail: String(e) });
      }
      refreshProviderCatalog(body);
    });
  });

  body.querySelectorAll<HTMLButtonElement>("[data-add-model]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const idx = parseInt(btn.dataset.addModel!, 10);
      openModelModal(idx, null);
    });
  });

  body.querySelectorAll<HTMLButtonElement>("[data-model-edit]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const [pi, mi] = btn.dataset.modelEdit!.split("|").map((n) => parseInt(n, 10));
      openModelModal(pi, mi);
    });
  });

  body.querySelectorAll<HTMLButtonElement>("[data-model-del]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const [pi, mi] = btn.dataset.modelDel!.split("|").map((n) => parseInt(n, 10));
      const prov = app.providers[pi];
      prov.models.splice(mi, 1);
      // Reindex validation map so badges don't shift onto wrong models.
      const reindexed = new Map<string, { checking: boolean; ok?: boolean; detail: string }>();
      modelValidation.forEach((v, k) => {
        const [kpi, kmi] = k.split(":").map((n) => parseInt(n, 10));
        if (kpi !== pi) { reindexed.set(k, v); return; }
        if (kmi === mi) return; // deleted
        if (kmi > mi) { reindexed.set(`${kpi}:${kmi - 1}`, v); return; }
        reindexed.set(k, v);
      });
      modelValidation.clear();
      reindexed.forEach((v, k) => modelValidation.set(k, v));
      refreshProviderCatalog(body);
    });
  });

  body.querySelectorAll<HTMLButtonElement>("[data-model-check]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const [pi, mi] = btn.dataset.modelCheck!.split("|").map((n) => parseInt(n, 10));
      const prov = app.providers[pi];
      const model = prov.models[mi];
      const key = `${pi}:${mi}`;
      modelValidation.set(key, { checking: true, detail: "checking…" });
      refreshProviderCatalog(body);
      try {
        const out = await api.providersValidate(prov.id, model.id);
        modelValidation.set(key, { checking: false, ok: out.ok, detail: out.detail });
      } catch (e) {
        modelValidation.set(key, { checking: false, ok: false, detail: String(e) });
      }
      refreshProviderCatalog(body);
    });
  });
}

function openProviderModal(editIdx: number | null): void {
  const modal = document.querySelector<HTMLDivElement>("#provider-modal")!;
  const title = document.querySelector<HTMLHeadingElement>("#provider-modal-title")!;
  const body = document.querySelector<HTMLDivElement>("#provider-modal-body")!;
  const isEdit = editIdx !== null;
  const existing = isEdit ? app.providers[editIdx!] : null;
  title.textContent = isEdit ? "Edit Provider" : "Add Provider";
  const idReadonly = isEdit ? "readonly" : "";
  const idVal = existing?.id ?? "";
  const nameVal = existing?.display_name ?? "";
  const baseVal = existing?.base_url ?? "";
  const protocolVal = existing?.protocol ?? "openai_compatible";
  // Determine auth type and values
  let authType: string = existing?.auth_type ?? "";
  let envVal = "";
  let rawVal = "";
  if (existing) {
    if (existing.auth_type === "raw") {
      authType = "raw";
      rawVal = existing.api_key ?? existing.api_key_env ?? "";
    } else if (existing.auth_type === "env_var") {
      authType = "env_var";
      envVal = existing.api_key_env ?? "";
    } else if (existing.auth_type === "none") {
      authType = "none";
    } else {
      // Auto-detect legacy
      const isEnv = /^[A-Z0-9_]{2,64}$/.test(existing.api_key_env ?? "");
      if (existing.api_key && existing.api_key.length > 0) {
        authType = "raw";
        rawVal = existing.api_key;
      } else if (isEnv) {
        authType = "env_var";
        envVal = existing.api_key_env ?? "";
      } else if (existing.api_key_env && existing.api_key_env.length > 0) {
        // Legacy raw stored in api_key_env
        authType = "raw";
        rawVal = existing.api_key_env;
      } else {
        authType = "env_var";
      }
    }
  } else {
    authType = "env_var";
  }
  const headers = existing?.headers && typeof existing.headers === "object" && !Array.isArray(existing.headers) ? Object.entries(existing.headers as Record<string,string>) : [] as [string,string][];
  const headersRows = headers.map(([k,v], i) => `
    <div class="flex space-x-2" data-header-row="${i}">
      <input data-header-key="${i}" value="${escapeAttr(k)}" placeholder="Header Name" class="flex-1 bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" />
      <input data-header-val="${i}" value="${escapeAttr(v)}" placeholder="Value" class="flex-1 bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" />
      <button type="button" data-header-remove="${i}" class="text-app-error text-xs px-2">×</button>
    </div>`).join("") || `<div class="text-xs text-app-textSecondary italic">No headers.</div>`;

  body.innerHTML = `
    <div class="provider-field"><label>Provider ID <span class="normal-case text-[10px] text-app-textSecondary">(lowercase, numbers, hyphens, underscores)</span></label>
      <input id="pm-id" value="${escapeAttr(idVal)}" ${idReadonly} placeholder="nvidia" class="font-mono ${isEdit?"bg-app-surface opacity-60":""}" /></div>
    <div class="provider-field"><label>Display Name</label><input id="pm-name" value="${escapeAttr(nameVal)}" placeholder="NVIDIA" /></div>
    <div class="provider-field"><label>Protocol</label><select id="pm-protocol"><option value="openai_compatible" ${protocolVal==="openai_compatible"?"selected":""}>OpenAI Compatible</option></select></div>
    <div class="provider-field"><label>Base URL</label><input id="pm-base" value="${escapeAttr(baseVal)}" placeholder="https://integrate.api.nvidia.com/v1" /></div>
    <div class="provider-field"><label>Authentication</label>
      <div class="flex space-x-4 text-xs">
        <label class="flex items-center space-x-1"><input type="radio" name="pm-auth" value="env_var" ${authType==="env_var"?"checked":""} /> <span>Environment Variable</span></label>
        <label class="flex items-center space-x-1"><input type="radio" name="pm-auth" value="raw" ${authType==="raw"?"checked":""} /> <span>API Key</span></label>
        <label class="flex items-center space-x-1"><input type="radio" name="pm-auth" value="none" ${authType==="none"?"checked":""} /> <span>None</span></label>
      </div>
      <div id="pm-auth-env" class="${authType==="env_var"?"":"hidden"} mt-2">
        <input id="pm-key-env" value="${escapeAttr(envVal)}" placeholder="NVIDIA_API_KEY" class="w-full bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" />
        <div class="text-[10px] text-app-textSecondary mt-1">Env var name resolved only in backend, never exposed.</div>
      </div>
      <div id="pm-auth-raw" class="${authType==="raw"?"":"hidden"} mt-2">
        <div class="flex space-x-2"><input id="pm-key-raw" value="${escapeAttr(rawVal)}" type="password" placeholder="nvapi-... or sk-..." class="flex-1 bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" /><button type="button" id="pm-key-toggle" class="text-xs px-2 py-1 border border-app-border rounded text-app-textSecondary">Show</button></div>
        <div class="text-[10px] text-app-textSecondary mt-1">Raw key stored securely, never shown in validation results.</div>
      </div>
      <div id="pm-auth-none" class="${authType==="none"?"":"hidden"} mt-2 text-[11px] text-app-textSecondary">No authentication — for local providers like Ollama.</div>
    </div>
    <div class="provider-field"><label>Headers (Optional)</label><div id="pm-headers" class="space-y-1">${headersRows}</div><button type="button" id="pm-add-header" class="mt-1 text-xs text-app-textSecondary hover:text-app-brand">+ Add Header</button></div>
    <div class="provider-field"><label>Discovered Models</label><div id="pm-discovered" class="text-xs max-h-32 overflow-y-auto border border-app-border rounded p-2 bg-app-bg">— not fetched —</div><button type="button" id="pm-fetch-models" class="mt-1 text-xs text-app-textSecondary hover:text-app-brand">Fetch Models</button></div>
    <div id="pm-status" class="text-xs font-mono mt-2"></div>
    <div class="flex items-center justify-between pt-3 border-t border-app-border">
      <button id="pm-check" type="button" class="text-xs px-3 py-1.5 border border-app-border rounded hover:border-app-brand text-app-textSecondary">Check Connection</button>
      <div class="flex space-x-2">
        <button id="pm-cancel" type="button" class="text-xs px-3 py-1.5 border border-app-border rounded text-app-textSecondary">Cancel</button>
        <button id="pm-save" type="button" class="text-xs px-4 py-1.5 bg-app-brand text-app-bg rounded font-semibold opacity-50" disabled>Save Provider</button>
      </div>
    </div>`;

  modal.classList.add("open");
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const lucide = (window as any).lucide; if (lucide?.createIcons) lucide.createIcons();

  let headersData: [string,string][] = [...headers];
  let _providerChecked = false; void _providerChecked;
  const statusEl = body.querySelector<HTMLDivElement>("#pm-status")!;

  const getAuthType = (): string => {
    const r = body.querySelector<HTMLInputElement>('input[name="pm-auth"]:checked');
    return r?.value ?? "env_var";
  };

  const updateSave = () => {
    const idEl = body.querySelector<HTMLInputElement>("#pm-id")!;
    const baseEl = body.querySelector<HTMLInputElement>("#pm-base")!;
    const auth = getAuthType();
    let keyOk = true;
    if (auth === "env_var") {
      const v = body.querySelector<HTMLInputElement>("#pm-key-env")!.value.trim();
      keyOk = v.length >= 2 && /^[A-Z0-9_]{2,64}$/.test(v);
    } else if (auth === "raw") {
      const v = body.querySelector<HTMLInputElement>("#pm-key-raw")!.value.trim();
      keyOk = v.length > 0;
    } else {
      keyOk = true;
    }
    const idOk = isValidProviderId(idEl.value.trim());
    const baseOk = baseEl.value.trim().length > 5 && /^https?:\/\//.test(baseEl.value.trim());
    const saveBtn = body.querySelector<HTMLButtonElement>("#pm-save")!;
    const canSave = idOk && baseOk && keyOk && (isEdit || !app.providers.some(p=>p.id===idEl.value.trim()));
    saveBtn.disabled = !canSave;
    saveBtn.style.opacity = canSave ? "1" : "0.5";
    if (!keyOk && auth === "env_var") {
      // keep status for env var format error? don't override checking status
    }
  };

  const updateAuthVisibility = () => {
    const auth = getAuthType();
    const envDiv = body.querySelector<HTMLDivElement>("#pm-auth-env")!;
    const rawDiv = body.querySelector<HTMLDivElement>("#pm-auth-raw")!;
    const noneDiv = body.querySelector<HTMLDivElement>("#pm-auth-none")!;
    envDiv.classList.toggle("hidden", auth !== "env_var");
    rawDiv.classList.toggle("hidden", auth !== "raw");
    noneDiv.classList.toggle("hidden", auth !== "none");
    _providerChecked = false;
    
    statusEl.textContent = "";
    updateSave();
  };

  body.querySelectorAll<HTMLInputElement>('input[name="pm-auth"]').forEach(r => r.addEventListener("change", updateAuthVisibility));
  body.querySelector<HTMLInputElement>("#pm-id")?.addEventListener("input", () => { _providerChecked = false; providerModalDirty = true; (body.querySelector("#pm-status") as HTMLElement).textContent=""; updateSave(); });
  body.querySelector<HTMLInputElement>("#pm-base")?.addEventListener("input", () => { _providerChecked = false; providerModalDirty = true; (body.querySelector("#pm-status") as HTMLElement).textContent=""; updateSave(); });
  body.querySelector<HTMLInputElement>("#pm-key-env")?.addEventListener("input", () => { _providerChecked = false; providerModalDirty = true; (body.querySelector("#pm-status") as HTMLElement).textContent=""; updateSave(); });
  body.querySelector<HTMLInputElement>("#pm-key-raw")?.addEventListener("input", () => { _providerChecked = false; providerModalDirty = true; (body.querySelector("#pm-status") as HTMLElement).textContent=""; updateSave(); });
  body.querySelector<HTMLButtonElement>("#pm-key-toggle")?.addEventListener("click", () => {
    const inp = body.querySelector<HTMLInputElement>("#pm-key-raw")!;
    inp.type = inp.type === "password" ? "text" : "password";
  });
  body.querySelector<HTMLButtonElement>("#pm-add-header")?.addEventListener("click", () => {
    headersData.push(["",""]);
    const container = body.querySelector<HTMLDivElement>("#pm-headers")!;
    container.innerHTML = headersData.map(([k,v], i) => `
      <div class="flex space-x-2" data-header-row="${i}">
        <input data-header-key="${i}" value="${escapeAttr(k)}" placeholder="Header Name" class="flex-1 bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" />
        <input data-header-val="${i}" value="${escapeAttr(v)}" placeholder="Value" class="flex-1 bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" />
        <button type="button" data-header-remove="${i}" class="text-app-error text-xs px-2">×</button>
      </div>`).join("");
    wireHeaderRows();
  });
  function wireHeaderRows() {
    body.querySelectorAll<HTMLInputElement>("[data-header-key]").forEach(el => {
      el.addEventListener("input", () => {
        const i = parseInt(el.dataset.headerKey!,10);
        headersData[i][0]=el.value;
        _providerChecked = false; providerModalDirty = true; (body.querySelector("#pm-status") as HTMLElement).textContent=""; updateSave();
      });
    });
    body.querySelectorAll<HTMLInputElement>("[data-header-val]").forEach(el => {
      el.addEventListener("input", () => {
        const i = parseInt(el.dataset.headerVal!,10);
        headersData[i][1]=el.value;
        _providerChecked = false; providerModalDirty = true; (body.querySelector("#pm-status") as HTMLElement).textContent=""; updateSave();
      });
    });
    body.querySelectorAll<HTMLButtonElement>("[data-header-remove]").forEach(btn=>{
      btn.addEventListener("click",()=>{
        const i=parseInt(btn.dataset.headerRemove!,10);
        headersData.splice(i,1);
        const cont = body.querySelector<HTMLDivElement>("#pm-headers")!;
        if(headersData.length===0) cont.innerHTML = `<div class="text-xs text-app-textSecondary italic">No headers.</div>`;
        else {
          cont.innerHTML = headersData.map(([k,v], idx) => `
            <div class="flex space-x-2" data-header-row="${idx}">
              <input data-header-key="${idx}" value="${escapeAttr(k)}" placeholder="Header Name" class="flex-1 bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" />
              <input data-header-val="${idx}" value="${escapeAttr(v)}" placeholder="Value" class="flex-1 bg-app-bg border border-app-border rounded px-2 py-1 text-xs font-mono" />
              <button type="button" data-header-remove="${idx}" class="text-app-error text-xs px-2">×</button>
            </div>`).join("");
          wireHeaderRows();
        }
        _providerChecked=false; updateSave();
      });
    });
  }
  wireHeaderRows();

  
  body.querySelector<HTMLButtonElement>("#pm-check")?.addEventListener("click", async () => {
    const btn = body.querySelector<HTMLButtonElement>("#pm-check")!;
    const base = (body.querySelector<HTMLInputElement>("#pm-base")!.value.trim());
    const id = (body.querySelector<HTMLInputElement>("#pm-id")!.value.trim());
    const auth = getAuthType();
    let key = "";
    if (auth === "env_var") key = (body.querySelector<HTMLInputElement>("#pm-key-env")!.value.trim());
    else if (auth === "raw") key = (body.querySelector<HTMLInputElement>("#pm-key-raw")!.value.trim());
    if (!isValidProviderId(id)) { statusEl.textContent="✗ Invalid Provider ID — use lowercase letters, numbers, hyphens, underscores (2-40 chars)"; statusEl.className="text-xs font-mono mt-2 text-app-error"; return; }
    if (!base) { statusEl.textContent="✗ Base URL required"; statusEl.className="text-xs font-mono mt-2 text-app-error"; return; }
    if (auth !== "none" && !key) { statusEl.textContent="✗ API Key required"; statusEl.className="text-xs font-mono mt-2 text-app-error"; return; }
    if (auth === "env_var" && !/^[A-Z0-9_]{2,64}$/.test(key)) { statusEl.textContent="✗ Invalid env var name — use UPPER_SNAKE (e.g. NVIDIA_API_KEY)"; statusEl.className="text-xs font-mono mt-2 text-app-error"; return; }
    btn.textContent="Checking…"; btn.disabled=true;
    statusEl.textContent="Checking…"; statusEl.className="text-xs font-mono mt-2 text-app-textSecondary";
    try {
      // For Check, we need to persist a temp provider to use the health checker which reads from file.
      // Instead, use direct checkProvider with the entered values to avoid file mutation.
      // Build a temporary descriptor via the health checker path: create a temp provider in memory and call checkProvider with base+key.
      // We'll call checkProvider directly with base and key (it will treat key as env var name if it looks like one, otherwise raw — but our health checker now handles raw).
      // To ensure raw vs env var is respected, we pass key as env var name if auth is env_var, otherwise raw.
      // The health checker will resolve accordingly if we construct descriptor correctly.
      // For simplicity, use the existing provider_check_connection path by temporarily saving.
      // But to avoid file mutation for Check, we use checkProvider directly.
      const out = await api.checkProvider(base, key, []);
      const ok = out.can_save;
      // For raw auth, checkProvider will treat key as raw if it doesn't look like env var; but for raw we want to ensure it uses raw key directly.
      // Our health checker now correctly handles both via is_env_var_name check, so this works.
      statusEl.textContent = ok ? `✓ API Response OK — ${out.message}` : `✗ Validation failed — ${out.message} — ${out.checks.map(c=> `${c.label}:${c.passed?"ok":"fail"}`).join(", ")}`;
      statusEl.className = `text-xs font-mono mt-2 ${ok ? "text-green-400" : "text-app-error"}`;
      _providerChecked = ok;
    } catch(e){
      statusEl.textContent = `✗ Validation failed — ${String(e).slice(0,200)}`;
      statusEl.className="text-xs font-mono mt-2 text-app-error";
      _providerChecked=false;
    } finally {
      btn.textContent="Check Connection"; btn.disabled=false;
      updateSave();
    }
  });

  body.querySelector<HTMLButtonElement>("#pm-fetch-models")?.addEventListener("click", async () => {
    const base = (body.querySelector<HTMLInputElement>("#pm-base")!.value.trim());
    const auth = getAuthType();
    let key = "";
    if (auth === "env_var") key = (body.querySelector<HTMLInputElement>("#pm-key-env")!.value.trim());
    else if (auth === "raw") key = (body.querySelector<HTMLInputElement>("#pm-key-raw")!.value.trim());
    const discEl = body.querySelector<HTMLDivElement>("#pm-discovered")!;
    discEl.textContent = "Fetching…";
    try {
      const out = await api.checkProvider(base, key, []);
      if (out.models_discovered && out.models_discovered.length > 0) {
        const existingIds2 = new Set((existing?.models ?? []).map(m => m.id));
        discEl.innerHTML = out.models_discovered.map(m => `<label class="flex items-center space-x-2 py-1"><input type="checkbox" value="${escapeAttr(m)}" class="pm-disc-check" ${existingIds2.has(m) ? "checked" : ""} /> <span class="font-mono text-xs">${escapeHtml(m)}</span></label>`).join("") + `<button type="button" id="pm-add-selected" class="mt-2 text-xs px-2 py-1 bg-app-brand text-app-bg rounded">Add Selected</button>`;
        discEl.querySelector<HTMLButtonElement>("#pm-add-selected")?.addEventListener("click", () => {
          // Checked boxes are merged into the saved provider by the Save handler.
          const n = discEl.querySelectorAll<HTMLInputElement>(".pm-disc-check:checked").length;
          if (!isEdit) {
            discEl.innerHTML += `<div class="text-xs text-app-warning mt-1">${n} model(s) checked — they will be saved when you click "Save Provider".</div>`;
            return;
          }
          const provIdx = editIdx!;
          let added = 0;
          discEl.querySelectorAll<HTMLInputElement>(".pm-disc-check:checked").forEach(ch => {
            const mid = ch.value;
            if (!app.providers[provIdx].models.some(m=>m.id===mid)) {
              app.providers[provIdx].models.push({ id: mid, display_name: mid, vision: false, tool_calling: true, streaming: true, context_window: null, max_output_tokens: null });
              added++;
            }
          });
          if (added > 0) {
            void api.providersSave(app.providers).then(() => {
              const settingsBody = document.querySelector<HTMLElement>("#modal-body");
              if (settingsBody) refreshProviderCatalog(settingsBody);
            }).catch(e => {
              discEl.innerHTML = `<div class="text-xs text-app-error">Save failed: ${escapeHtml(String(e).slice(0,150))}</div>` + discEl.innerHTML;
            });
            discEl.innerHTML = `<div class="text-xs text-green-400">Added ${added} model(s) — click "Save Provider" to persist.</div>` + discEl.innerHTML;
          }
        });
      } else {
        discEl.textContent = out.models_discovered ? "No models discovered (endpoint may not support /models). Add manually." : "No models returned.";
      }
    } catch(e) {
      discEl.textContent = `Fetch failed: ${String(e).slice(0,200)}`;
    }
  });

  body.querySelector<HTMLButtonElement>("#pm-cancel")?.addEventListener("click", () => closeProviderModal());
  body.querySelector<HTMLButtonElement>("#pm-save")?.addEventListener("click", async () => {
    const id = (body.querySelector<HTMLInputElement>("#pm-id")!.value.trim());
    const name = (body.querySelector<HTMLInputElement>("#pm-name")!.value.trim() || id);
    const protocol = (body.querySelector<HTMLSelectElement>("#pm-protocol")!.value || "openai_compatible");
    const base = (body.querySelector<HTMLInputElement>("#pm-base")!.value.trim());
    const auth = getAuthType();
    let keyEnv = "";
    let rawKey: string | null = null;
    if (auth === "env_var") keyEnv = (body.querySelector<HTMLInputElement>("#pm-key-env")!.value.trim());
    else if (auth === "raw") rawKey = (body.querySelector<HTMLInputElement>("#pm-key-raw")!.value.trim());
    const hd: Record<string,string> = {};
    headersData.forEach(([k,v])=>{ if(k.trim()) hd[k.trim()]=v; });
    if (!isValidProviderId(id)) { statusEl.textContent="✗ Invalid Provider ID"; statusEl.className="text-xs font-mono mt-2 text-app-error"; return; }
    if (!base) { statusEl.textContent="✗ Base URL required"; return; }
    if (auth !== "none" && !(keyEnv || rawKey)) { statusEl.textContent="✗ API Key required"; return; }
    if (auth === "env_var" && keyEnv && !/^[A-Z0-9_]{2,64}$/.test(keyEnv)) { statusEl.textContent="✗ Invalid env var name"; statusEl.className="text-xs font-mono mt-2 text-app-error"; return; }
    // Collect selected discovered models (if any) — fixes “MODELS (0)” bug
    const discEl2 = body.querySelector<HTMLDivElement>("#pm-discovered");
    const selectedIds = discEl2 ? Array.from(discEl2.querySelectorAll<HTMLInputElement>(".pm-disc-check:checked")).map(cb => cb.value) : [];
    const discoveredAll = discEl2 ? Array.from(discEl2.querySelectorAll<HTMLInputElement>(".pm-disc-check")).map(cb => cb.value) : [];
    const existingModels = existing?.models ?? [];
    let mergedModels = [...existingModels];
    for (const mid of selectedIds) {
      if (!mergedModels.some(m => m.id === mid)) {
        mergedModels.push({ id: mid, display_name: mid, vision: false, tool_calling: true, streaming: true, context_window: null, max_output_tokens: null });
      }
    }
    for (const mid of discoveredAll) {
      const wasInExisting = existingModels.some(m => m.id === mid);
      const isChecked = selectedIds.includes(mid);
      if (wasInExisting && !isChecked) {
        mergedModels = mergedModels.filter(m => m.id !== mid);
      }
    }
    const seenIds = new Set<string>();
    mergedModels = mergedModels.filter(m => {
      if (seenIds.has(m.id)) return false;
      seenIds.add(m.id);
      return true;
    });
    const entry: ProviderEntryDto = {
      id, display_name: name, protocol, base_url: base, api_key_env: auth === "env_var" ? keyEnv : "",
      auth_type: auth, api_key: auth === "raw" ? rawKey : null,
      headers: Object.keys(hd).length ? hd as unknown as ProviderEntryDto["headers"] : null,
      extra_body: existing?.extra_body ?? null,
      models: mergedModels
    };
    if (isEdit) {
      app.providers[editIdx!] = entry;
    } else {
      if (app.providers.some(p=>p.id===id)) { statusEl.textContent=`✗ Provider ID '${id}' already exists`; statusEl.className="text-xs font-mono mt-2 text-app-error"; return; }
      app.providers.push(entry);
      providerValidation.set(app.providers.length-1, { checking:false, ok: true, detail: "saved" });
    }
    try {
      await api.providersSave(app.providers);
      if (isEdit) providerValidation.set(editIdx!, { checking:false, ok: true, detail:"saved" });
      closeProviderModal(true);
      const settingsBody = document.querySelector<HTMLElement>("#modal-body");
      if (settingsBody) refreshProviderCatalog(settingsBody);
    } catch(e){
      statusEl.textContent = `Save failed: ${String(e)}`;
      statusEl.className="text-xs font-mono mt-2 text-app-error";
    }
  });

  updateSave();
}

function closeProviderModal(force = false): void {
  if (!force && !confirmDiscard("provider")) return;
  providerModalDirty = false;
  const modal = document.querySelector<HTMLDivElement>("#provider-modal")!;
  modal.classList.remove("open");
}

function openModelModal(providerIdx: number, modelIdx: number | null): void {
  const modal = document.querySelector<HTMLDivElement>("#model-modal")!;
  const title = document.querySelector<HTMLHeadingElement>("#model-modal-title")!;
  const body = document.querySelector<HTMLDivElement>("#model-modal-body")!;
  const prov = app.providers[providerIdx];
  const isEdit = modelIdx !== null;
  const existing = isEdit ? prov.models[modelIdx!] : null;
  title.textContent = isEdit ? "Edit Model" : "Add Model";
  const modelIdVal = existing?.id ?? "";
  const displayVal = existing?.display_name ?? "";
  const toolsVal = existing ? existing.tool_calling : true;
  const visionVal = existing ? existing.vision : false;
  const streamingVal = existing ? existing.streaming : true;
  const ctxVal = existing?.context_window ?? "";

  body.innerHTML = `
    <div class="text-xs text-app-textSecondary mb-2">Provider: <span class="font-mono text-app-textPrimary">${escapeHtml(prov.display_name || prov.id)}</span> <span class="font-mono text-[10px] text-app-textSecondary">(${escapeHtml(prov.id)})</span></div>
    <div class="provider-field"><label>Model ID <span class="text-[10px] normal-case text-app-textSecondary">(sent to API)</span></label><input id="mm-id" value="${escapeAttr(modelIdVal)}" placeholder="gpt-4o or meta/llama-3.1-70b" /></div>
    <div class="provider-field"><label>Display Name <span class="text-[10px] normal-case text-app-textSecondary">(shown in UI)</span></label><input id="mm-display" value="${escapeAttr(displayVal)}" placeholder="Llama 3.1 70B" /></div>
    <div class="provider-field"><label>Capabilities</label>
      <label class="flex items-center space-x-2 text-xs"><input type="checkbox" id="mm-tools" ${toolsVal?"checked":""} /> <span>Tools</span></label>
      <label class="flex items-center space-x-2 text-xs"><input type="checkbox" id="mm-vision" ${visionVal?"checked":""} /> <span>Vision</span></label>
      <label class="flex items-center space-x-2 text-xs"><input type="checkbox" id="mm-streaming" ${streamingVal?"checked":""} /> <span>Streaming</span></label>
    </div>
    <div class="provider-field"><label>Context Window (optional)</label><input id="mm-ctx" value="${escapeAttr(String(ctxVal))}" placeholder="128000" type="number" /></div>
    <div id="mm-status" class="text-xs font-mono mt-2"></div>
    <div class="flex items-center justify-between pt-3 border-t border-app-border">
      <button id="mm-check" type="button" class="text-xs px-3 py-1.5 border border-app-border rounded hover:border-app-brand text-app-textSecondary">Check API Response</button>
      <div class="flex space-x-2">
        <button id="mm-cancel" type="button" class="text-xs px-3 py-1.5 border border-app-border rounded text-app-textSecondary">Cancel</button>
        <button id="mm-save" type="button" class="text-xs px-4 py-1.5 bg-app-brand text-app-bg rounded font-semibold opacity-50" disabled>Save Model</button>
      </div>
    </div>`;

  modal.classList.add("open");
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const lucide = (window as any).lucide; if (lucide?.createIcons) lucide.createIcons();

  let modelChecked = false;
  const statusEl = body.querySelector<HTMLDivElement>("#mm-status")!;
  const updateSave = () => {
    const idOk = (body.querySelector<HTMLInputElement>("#mm-id")!.value.trim().length > 0);
    const btn = body.querySelector<HTMLButtonElement>("#mm-save")!;
    const can = idOk;
    btn.disabled = !can;
    btn.style.opacity = can ? "1" : "0.5";
  };
  body.querySelector<HTMLInputElement>("#mm-id")?.addEventListener("input", ()=>{ modelChecked=false; modelModalDirty=true; statusEl.textContent=""; updateSave(); });
  body.querySelector<HTMLButtonElement>("#mm-check")?.addEventListener("click", async () => {
    const mid = (body.querySelector<HTMLInputElement>("#mm-id")!.value.trim());
    if (!mid) { statusEl.textContent="✗ Model ID required"; statusEl.className="text-xs font-mono mt-2 text-app-error"; return; }
    const btn = body.querySelector<HTMLButtonElement>("#mm-check")!;
    btn.textContent="Checking…"; btn.disabled=true;
    statusEl.textContent="Checking…"; statusEl.className="text-xs font-mono mt-2 text-app-textSecondary";
    try {
      const out = await api.providersValidate(prov.id, mid);
      statusEl.textContent = out.ok ? `✓ API Response OK — ${out.detail.slice(0,120)}` : `✗ Validation failed — ${out.detail.slice(0,160)} [${out.class ?? "unknown"}]`;
      statusEl.className = `text-xs font-mono mt-2 ${out.ok ? "text-green-400" : "text-app-error"}`;
      modelChecked = out.ok;
    } catch(e){
      statusEl.textContent = `✗ Validation failed — ${String(e).slice(0,200)}`;
      statusEl.className="text-xs font-mono mt-2 text-app-error";
      modelChecked=false;
    } finally {
      btn.textContent="Check API Response"; btn.disabled=false;
      updateSave();
    }
  });
  body.querySelector<HTMLButtonElement>("#mm-cancel")?.addEventListener("click", () => {
    closeModelModal();
  });
  body.querySelector<HTMLButtonElement>("#mm-save")?.addEventListener("click", async () => {
    const mid = (body.querySelector<HTMLInputElement>("#mm-id")!.value.trim());
    const display = (body.querySelector<HTMLInputElement>("#mm-display")!.value.trim() || mid);
    if (!mid) { statusEl.textContent="✗ Model ID required"; return; }
    if (!modelChecked) {
      console.warn("saving model without validation");
    }
    const draft = {
      id: mid,
      display_name: display,
      vision: (body.querySelector<HTMLInputElement>("#mm-vision")!.checked),
      tool_calling: (body.querySelector<HTMLInputElement>("#mm-tools")!.checked),
      streaming: (body.querySelector<HTMLInputElement>("#mm-streaming")!.checked),
      context_window: (()=>{ const v=(body.querySelector<HTMLInputElement>("#mm-ctx")!.value.trim()); const n=parseInt(v,10); return isNaN(n)? null: n; })(),
      max_output_tokens: null,
    };
    if (isEdit) {
      prov.models[modelIdx!] = draft as typeof prov.models[number];
    } else {
      if (prov.models.some(m=>m.id===mid)) {
        statusEl.textContent=`✗ Model ID '${mid}' already exists in this provider`;
        statusEl.className="text-xs font-mono mt-2 text-app-error";
        return;
      }
      prov.models.push(draft as typeof prov.models[number]);
    }
    try {
      await api.providersSave(app.providers);
      const key = `${providerIdx}:${isEdit? modelIdx! : prov.models.findIndex(m=>m.id===mid)}`;
      modelValidation.set(key, { checking:false, ok:true, detail:"validated" });
      closeModelModal(true);
      const settingsBody = document.querySelector<HTMLElement>("#modal-body");
      if (settingsBody) refreshProviderCatalog(settingsBody);
    } catch(e){
      statusEl.textContent = `Save failed: ${String(e)}`;
      statusEl.className="text-xs font-mono mt-2 text-app-error";
    }
  });
  updateSave();
}

function closeModelModal(force = false): void {
  if (!force && !confirmDiscard("model")) return;
  modelModalDirty = false;
  const modal = document.querySelector<HTMLDivElement>("#model-modal")!;
  modal.classList.remove("open");
}

// ----- Sidebar collapse (spec §30) -----

const SIDEBAR_KEY = "aether_sidebar_collapsed";

function isSidebarCollapsed(): boolean {
  return localStorage.getItem(SIDEBAR_KEY) === "1";
}

function setSidebarCollapsed(collapsed: boolean): void {
  localStorage.setItem(SIDEBAR_KEY, collapsed ? "1" : "0");
  const sidebar = document.querySelector<HTMLDivElement>("#workspace-sidebar")!;
  if (collapsed) sidebar.classList.add("collapsed");
  else sidebar.classList.remove("collapsed");
  updateMainMargins();
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const lucide = (window as any).lucide; if (lucide?.createIcons) lucide.createIcons();
}

function updateMainMargins(): void {
  const sidebar = document.querySelector<HTMLDivElement>("#workspace-sidebar")!;
  const isHome = document.querySelector<HTMLDivElement>("#workspace-home")?.style.display !== "none";
  const header = document.querySelector<HTMLElement>("#app-header");
  const stream = document.querySelector<HTMLDivElement>("#stream");
  const inputBar = document.querySelector<HTMLDivElement>("#input-bar");
  const collapsed = sidebar.classList.contains("collapsed");
  const visible = sidebar.style.display !== "none" && !isHome;
  const left = !visible ? "0" : collapsed ? "48px" : "256px";
  if (header) header.style.marginLeft = left;
  if (stream) stream.style.marginLeft = left;
  if (inputBar) inputBar.style.left = left;
}

function initSidebar(): void {
  const collapsed = isSidebarCollapsed();
  if (collapsed) document.querySelector<HTMLDivElement>("#workspace-sidebar")?.classList.add("collapsed");
  updateMainMargins();
  document.querySelector("#sidebar-toggle")?.addEventListener("click", () => setSidebarCollapsed(true));
  document.querySelector("#sidebar-expand")?.addEventListener("click", () => setSidebarCollapsed(false));
  // also wire top-bar toggle if present
  // Ensure layout updates on window resize
  window.addEventListener("resize", updateMainMargins);
}

function wireSettings(body: HTMLElement, originalCfg: DesktopConfig) {
  // ----- Skills list -----
  void api.listSkills().then((skills) => {
    const out = body.querySelector("#skill-list");
    if (!out) return;
    if (skills.length === 0) {
      out.textContent = "No skills found. Drop SKILL.md into ~/.aether/skills/ or your repo.";
    } else {
      out.innerHTML = skills
        .slice(0, 8)
        .map((s) => `<div>• <code class="font-mono">${escapeHtml(s.id)}</code> — ${escapeHtml(s.description.slice(0, 80))}</div>`)
        .join("");
      if (skills.length > 8) out.innerHTML += `<div class="mt-1">+${skills.length - 8} more</div>`;
    }
  }).catch(() => { /* ignore */ });

  // ----- Provider health check -----
  body.querySelector<HTMLButtonElement>("#hc-run")?.addEventListener("click", async () => {
    const url = body.querySelector<HTMLInputElement>("#hc-url")?.value.trim() ?? "";
    const env = body.querySelector<HTMLInputElement>("#hc-env")?.value.trim() ?? "OPENAI_API_KEY";
    const model = body.querySelector<HTMLInputElement>("#hc-model")?.value.trim() ?? "";
    const out = body.querySelector("#hc-out");
    if (!out || !url) { if (out) out.textContent = "URL is required."; return; }
    out.textContent = "Checking…";
    try {
      const r = await api.checkProvider(url, env, model ? [model] : []);
      const lines = r.checks.map((c) => `${c.passed ? "✓" : "✗"} ${c.label}: ${c.detail}`).join("\n");
      out.textContent = `${r.message}\n\n${lines}\n\nStatus: ${r.status} · Latency: ${r.total_latency_ms}ms · Can save: ${r.can_save}`;
    } catch (e) {
      out.textContent = `Check failed: ${String(e)}`;
    }
  });

  // ----- Snapshots -----
  async function refreshSnapshotList() {
    const sid = body.querySelector<HTMLInputElement>("#snap-session")?.value.trim() ?? "";
    const out = body.querySelector("#snap-list-out");
    if (!out) return;
    if (!sid) { out.textContent = "Enter a session id."; return; }
    try {
      const list = await api.listSnapshots(sid);
      if (list.length === 0) { out.textContent = "(no snapshots for this session yet)"; return; }
      out.innerHTML = list.map((s) => {
        const files = s.files.length ? `${s.files.length} files` : "0 files";
        return `<div class="border-b border-app-border/50 py-1 cursor-pointer hover:bg-app-hover px-1" data-snap-id="${escapeAttr(s.id)}">
          <div class="text-app-textPrimary">${escapeHtml(s.id)} <span class="text-app-textSecondary">${escapeHtml(s.trigger)} · ${files}</span></div>
          <div class="text-app-textSecondary">${escapeHtml(s.timestamp)} · ${escapeHtml(s.agent_id ?? "")}</div>
        </div>`;
      }).join("");
      out.querySelectorAll<HTMLDivElement>("[data-snap-id]").forEach((d) =>
        d.addEventListener("click", async () => {
          const id = d.dataset.snapId!;
          const r = await api.restoreSnapshot(sid, id);
          out.innerHTML = `<div class="validation-${r.success ? "ok" : "err"}">${escapeHtml(r.message)} (${r.files_restored} files)</div>` + out.innerHTML;
        }),
      );
    } catch (e) {
      out.textContent = `Failed: ${String(e)}`;
    }
  }
  body.querySelector<HTMLButtonElement>("#snap-list")?.addEventListener("click", () => void refreshSnapshotList());
  body.querySelector<HTMLButtonElement>("#snap-undo")?.addEventListener("click", async () => {
    const sid = body.querySelector<HTMLInputElement>("#snap-session")?.value.trim() ?? "";
    const out = body.querySelector("#snap-list-out");
    if (!out || !sid) return;
    try {
      const r = await api.snapshotUndo(sid);
      out.innerHTML = `<div class="validation-${r.success ? "ok" : "err"}">${escapeHtml(r.message)} (${r.files_restored} files)</div>` + (out.innerHTML ?? "");
      await refreshSnapshotList();
    } catch (e) { out.textContent = String(e); }
  });
  body.querySelector<HTMLButtonElement>("#snap-redo")?.addEventListener("click", async () => {
    const sid = body.querySelector<HTMLInputElement>("#snap-session")?.value.trim() ?? "";
    const out = body.querySelector("#snap-list-out");
    if (!out || !sid) return;
    try {
      const r = await api.snapshotRedo(sid);
      out.innerHTML = `<div class="validation-${r.success ? "ok" : "err"}">${escapeHtml(r.message)} (${r.files_restored} files)</div>` + (out.innerHTML ?? "");
      await refreshSnapshotList();
    } catch (e) { out.textContent = String(e); }
  });

  // ----- SonarQube code-analysis (v0.14) -----
  wireAnalysisPanel(body);

  // ----- Provider catalog (v0.17) -----
  wireProviderCatalog(body);

  // ----- Live opacity preview -----
  const opacitySlider = body.querySelector<HTMLInputElement>("#bg-opacity");
  const opacityLabel = body.querySelector<HTMLSpanElement>("#bg-opacity-label");
  opacitySlider?.addEventListener("input", () => {
    if (opacityLabel) opacityLabel.textContent = `${opacitySlider.value}%`;
    applyBackgroundFromSettings(body);
  });

  // ----- Live display-mode preview -----
  body.querySelector<HTMLSelectElement>("#bg-mode")?.addEventListener("change", () => {
    applyBackgroundFromSettings(body);
  });

  // ----- Background enable toggle (live) -----
  body.querySelector<HTMLDivElement>("#bg-toggle")?.addEventListener("click", () => {
    const t = body.querySelector<HTMLDivElement>("#bg-toggle")!;
    t.classList.toggle("on");
    const label = t.querySelector(".label");
    if (label) label.textContent = t.classList.contains("on") ? "Background Image ON" : "Background Image OFF";
    applyBackgroundFromSettings(body);
  });

  // ----- Background upload -----
  const fileInput = body.querySelector<HTMLInputElement>("#bg-upload-input");
  body.querySelector<HTMLButtonElement>("#bg-upload")?.addEventListener("click", () => fileInput?.click());
  fileInput?.addEventListener("change", async () => {
    const f = fileInput.files?.[0];
    if (!f) return;
    const arr = new Uint8Array(await f.arrayBuffer());
    const validation = body.querySelector<HTMLDivElement>("#bg-validation")!;
    try {
      const result = await api.setBackgroundImage(Array.from(arr));
      validation.classList.remove("validation-err");
      validation.classList.add("validation-ok");
      validation.style.display = "block";
      validation.textContent = result.message;
      // Refresh on-disk setting then re-render preview.
      originalCfg.appearance.background_image = result.saved_path ?? null;
      await refreshBackgroundPreview(body);
    } catch (e) {
      validation.classList.remove("validation-ok");
      validation.classList.add("validation-err");
      validation.style.display = "block";
      validation.textContent = String(e);
    }
  });

  // ----- Reset to default -----
  body.querySelector<HTMLButtonElement>("#bg-reset")?.addEventListener("click", async () => {
    originalCfg.appearance.background_image = null;
    const validation = body.querySelector<HTMLDivElement>("#bg-validation")!;
    validation.style.display = "none";
    validation.textContent = "";
    await refreshBackgroundPreview(body);
  });

  // ----- Save -----
  body.querySelector<HTMLButtonElement>("#save-btn")?.addEventListener("click", async () => {
    const status = body.querySelector<HTMLSpanElement>("#save-status")!;
    status.textContent = "Saving…";

    const appearance = collectAppearance(body, originalCfg.appearance);

    try {
      // Persist the provider catalog (v0.17).
      await api.providersSave(app.providers);
      // Persist appearance/frontend to config.toml.
      const path = await api.writeConfig({
        agent: originalCfg.agent,
        models: originalCfg.models,
        frontend: originalCfg.frontend,
        appearance,
      });
      status.textContent = `Saved to ${path}`;
      await refreshBackgroundPreview(body);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const lucide = (window as any).lucide;
      if (lucide?.createIcons) lucide.createIcons();
    } catch (e) {
      status.textContent = `Save failed: ${String(e)}`;
    }
  });
}

// ---------------------------------------------------------------------------
// Code-analysis panel — SonarQube capability (v0.14)
// ---------------------------------------------------------------------------
//
// The panel keeps a single run-state machine: idle → probing → analyzing →
// done | error. Progress arrives via the `analysis-progress` event the backend
// emits around `analysis_run`. Results render as a severity histogram +
// affected file list + top findings; nothing is editable here because findings
// are advisory input for the controller, not user-managed configuration.

const sevColor: Record<string, string> = {
  blocker: "text-red-400",
  high: "text-orange-400",
  medium: "text-yellow-400",
  low: "text-blue-400",
  info: "text-app-textSecondary",
};

let analysisMode: "run" | "scanner" = "run";

function renderSeverityDist(r: import("./api").AnalysisReport): string {
  const total = r.finding_count;
  const bar = (label: string, n: number, cls: string): string =>
    n > 0 ? `<span class="${cls} font-mono">${label} ${n}</span>` : "";
  return `<div class="flex items-center space-x-3 text-xs flex-wrap">
    <span class="text-app-textPrimary font-semibold">${total} findings</span>
    ${bar("blocker", r.blocker, sevColor.blocker)}
    ${bar("high", r.high, sevColor.high)}
    ${bar("medium", r.medium, sevColor.medium)}
    ${bar("low", r.low, sevColor.low)}
    ${bar("info", r.info, sevColor.info)}
  </div>`;
}

function renderFindingsTable(findings: import("./api").AnalysisFinding[], max: number): string {
  const rows = findings.slice(0, max).map((f) => {
    const loc = f.path ? `${f.path}:${f.start_line}` : "(project)";
    return `<tr class="border-b border-app-border/40">
      <td class="py-1 pr-2 whitespace-nowrap ${sevColor[f.severity] ?? ""}">${escapeHtml(f.severity)}</td>
      <td class="py-1 pr-2 font-mono text-xs">${escapeHtml(f.rule)}</td>
      <td class="py-1 pr-2 text-xs">${escapeHtml(f.message)}</td>
      <td class="py-1 font-mono text-xs whitespace-nowrap">${escapeHtml(loc)}</td>
    </tr>`;
  }).join("");
  const note = findings.length > max
    ? `<div class="text-xs text-app-textSecondary mt-1">…and ${findings.length - max} more (see full report)</div>`
    : "";
  return `<table class="w-full text-left">${rows}</table>${note}`;
}

function renderReportCard(r: import("./api").AnalysisReport): string {
  const files = r.affected_files.slice(0, 8).join(", ") + (r.affected_files.length > 8 ? " …" : "");
  return `<div class="border border-app-border rounded p-2 bg-app-bg mt-2">
    <div class="flex items-center justify-between flex-wrap">
      ${renderSeverityDist(r)}
      <span class="text-xs text-app-textSecondary font-mono">${escapeHtml(r.at)}</span>
    </div>
    <div class="text-xs text-app-textSecondary mt-1">
      project <code class="font-mono">${escapeHtml(r.project)}</code>
      ${r.label ? ` · <span class="text-app-textPrimary">${escapeHtml(r.label)}</span>` : ""}
    </div>
    ${files ? `<div class="text-xs text-app-textSecondary mt-1">${escapeHtml(files)}</div>` : ""}
    <div class="mt-2 max-h-48 overflow-y-auto">${renderFindingsTable(r.findings, 10)}</div>
  </div>`;
}

function wireAnalysisPanel(body: HTMLElement): void {
  const statusEl = body.querySelector<HTMLDivElement>("#sa-status")!;
  const reportEl = body.querySelector<HTMLDivElement>("#sa-report")!;
  const modeBtn = body.querySelector<HTMLButtonElement>("#sa-mode")!;
  let progressUnlisten: (() => void) | null = null;

  void events.onAnalysisProgress((p) => {
    if (p.stage === "probing") statusEl.textContent = "Probing SonarQube server…";
    else if (p.stage === "analyzing") statusEl.textContent = "Analyzing project…";
    else if (p.stage === "done") {
      statusEl.textContent = `Done — ${p.findings ?? 0} findings`;
      if (progressUnlisten) { progressUnlisten(); progressUnlisten = null; }
    } else if (p.stage === "error") {
      statusEl.textContent = `Error: ${p.message ?? "unknown"}`;
      if (progressUnlisten) { progressUnlisten(); progressUnlisten = null; }
    }
  }).then((u) => { progressUnlisten = u; });

  modeBtn.addEventListener("click", () => {
    analysisMode = analysisMode === "run" ? "scanner" : "run";
    modeBtn.textContent = analysisMode;
    modeBtn.title = "Toggle scan mode: run (fetch current results) / scanner (launch sonar-scanner first)";
  });

  body.querySelector<HTMLButtonElement>("#sa-run")?.addEventListener("click", async () => {
    const root = body.querySelector<HTMLInputElement>("#sa-root")?.value.trim() ?? "";
    const baseUrl = body.querySelector<HTMLInputElement>("#sa-url")?.value.trim() ?? "";
    const tokenEnv = body.querySelector<HTMLInputElement>("#sa-token-env")?.value.trim() ?? "";
    if (!root) { statusEl.textContent = "Enter a project root first."; return; }

    statusEl.textContent = "Checking availability…";
    try {
      const avail = await api.analysisCheck(baseUrl || undefined, tokenEnv || undefined);
      if (!avail.available) {
        statusEl.textContent = `Unavailable: ${avail.detail}`;
        return;
      }
      statusEl.textContent = "Running analysis…";
      const r = await api.analysisRun(root, {
        mode: analysisMode,
        baseUrl: baseUrl || undefined,
        tokenEnv: tokenEnv || undefined,
        label: "manual-run",
      });
      if (r.success && r.report) {
        statusEl.textContent = `${r.message}`;
        reportEl.innerHTML = renderReportCard(r.report);
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const lucide = (window as any).lucide;
        if (lucide?.createIcons) lucide.createIcons();
      } else {
        statusEl.textContent = r.message;
        reportEl.innerHTML = "";
      }
    } catch (e) {
      statusEl.textContent = `Analysis failed: ${String(e)}`;
      reportEl.innerHTML = "";
    }
  });
}

// ----- Background application (spec §16-§17) -----

let cachedBackground: { data: string; contentType: string } | null = null;

async function refreshBackgroundPreview(body: HTMLElement): Promise<void> {
  const img = body.querySelector<HTMLImageElement>("#bg-preview-img");
  try {
    const payload = await api.getBackground();
    if (payload.data_base64) {
      const url = `data:${payload.content_type};base64,${payload.data_base64}`;
      if (img) img.src = url;
      cachedBackground = { data: payload.data_base64, contentType: payload.content_type };
    } else {
      if (img) img.src = "";
      cachedBackground = null;
    }
  } catch {
    if (img) img.src = "";
    cachedBackground = null;
  }
}

function applyBackgroundFromSettings(body: HTMLElement): void {
  const enabled = body.querySelector<HTMLDivElement>("#bg-toggle")?.classList.contains("on") ?? true;
  const opacity = parseInt(body.querySelector<HTMLInputElement>("#bg-opacity")?.value ?? "60", 10);
  const mode = body.querySelector<HTMLSelectElement>("#bg-mode")?.value ?? "fill";
  applyBackgroundLayer(enabled, isNaN(opacity) ? 60 : opacity, mode);
}

function objectFitForMode(mode: string): string {
  switch (mode) {
    case "fit": return "contain";
    case "stretch": return "fill";
    case "center": return "none";
    default: return "cover";
  }
}

function applyBackgroundLayer(enabled: boolean, opacityPct: number, mode = "fill"): void {
  const img = document.querySelector<HTMLImageElement>("#bg-img");
  const fade = document.querySelector<HTMLDivElement>("#bg-fade");
  if (!img || !fade) return;
  if (!enabled || !cachedBackground) {
    img.style.display = "none";
    fade.style.opacity = "1";
    return;
  }
  if (!img.src) img.src = `data:${cachedBackground.contentType};base64,${cachedBackground.data}`;
  img.style.display = "block";
  img.style.objectFit = objectFitForMode(mode);
  img.style.opacity = String(opacityPct / 100);
  // The fade layer keeps the UI readable when the image is bright.
  fade.style.opacity = String(1 - opacityPct / 100 * 0.6);
}

function renderHistory(rows: import("./api").SessionRow[]): string {
  if (rows.length === 0) return `<div class="text-app-textSecondary text-sm">No past sessions yet.</div>`;
  return `<ul class="space-y-2">
    ${rows.map((r) => `<li class="bg-app-bg border border-app-border rounded-lg p-3 cursor-pointer hover:border-app-brand/60" data-sid="${escapeAttr(r.id)}">
      <div class="text-sm text-app-textPrimary">${escapeHtml((r.task ?? "(no task)").slice(0, 120))}</div>
      <div class="text-xs text-app-textSecondary mt-1 font-mono">${escapeHtml(r.id)} · ${escapeHtml(r.created_at)}</div>
    </li>`).join("")}
  </ul>`;
}

function wireHistory(body: HTMLElement) {
  body.querySelectorAll<HTMLLIElement>("[data-sid]").forEach((li) => {
    li.addEventListener("click", async () => {
      const sid = li.dataset.sid!;
      li.classList.add("border-app-brand");
      const msgs = await api.getSessionMessages(sid);
      body.innerHTML = `<div class="space-y-2 text-sm">
        <button id="back-btn" class="text-app-textSecondary hover:text-app-brand text-xs">← Back</button>
        ${msgs.map((m) => `<div class="border-l-2 ${m.role === "user" ? "border-app-brand" : "border-app-border"} pl-3 py-1">
          <div class="text-xs uppercase text-app-textSecondary">${escapeHtml(m.role)}</div>
          <pre class="whitespace-pre-wrap font-mono text-xs text-app-textPrimary">${escapeHtml(m.content)}</pre>
        </div>`).join("")}
      </div>`;
      body.querySelector<HTMLButtonElement>("#back-btn")?.addEventListener("click", () => openModal("history"));
    });
  });
}

// ----- Boot -----

async function boot() {
  // Global crash guards — any unhandled exception must become an in-app error, never a terminated window.
  window.addEventListener("error", (e) => {
    console.error("window error", e.error ?? e.message);
    try {
      const s = current();
      s.blocks.push({ id: newId(s), kind: "error", text: `Application error: ${String(e.message ?? e.error ?? e).slice(0, 400)}\nThe app remains open. Check Settings or try again.` });
      renderAll();
    } catch {}
  });
  window.addEventListener("unhandledrejection", (e) => {
    console.error("unhandledrejection", e.reason);
    try {
      const s = current();
      s.blocks.push({ id: newId(s), kind: "error", text: `Unexpected error: ${String(e.reason ?? e).slice(0, 400)}\nThe app remains open.` });
      renderAll();
    } catch {}
    e.preventDefault();
  });

  app.tabs.push(createSession("Welcome"));
  renderAll();
  initSidebar();

  // Apply background on launch (spec §16): we don't wait for the Settings modal.
  try {
    const r = await api.readConfig();
    const appearance = r.config.appearance ?? {
      background_enabled: true,
      background_opacity: 60,
      background_image: null,
      background_mode: "fill",
    };
    try {
      const payload = await api.getBackground();
      if (payload.data_base64) {
        cachedBackground = { data: payload.data_base64, contentType: payload.content_type };
      }
    } catch { /* keep null; layer renders solid */ }
    applyBackgroundLayer(appearance.background_enabled, appearance.background_opacity, appearance.background_mode ?? "fill");
  } catch {
    /* ignore */
  }

  // Top-bar buttons (stable <button> wrappers with ids).
  document.querySelector("#new-tab-btn")!.addEventListener("click", newTab);
  document.querySelector("#open-settings-btn")!.addEventListener("click", () => openModal("settings"));
  document.querySelector("#open-history-btn")!.addEventListener("click", () => openModal("history"));
  document.querySelector("#modal-close")!.addEventListener("click", closeModal);

  // Event delegation for tabs (their inner DOM is replaced on every renderTabs).
  document.querySelector("#tabs")!.addEventListener("click", (e) => {
    const t = e.target as HTMLElement;
    const closeBtn = t.closest<HTMLElement>("[data-close-tab]");
    if (closeBtn) {
      const idx = parseInt(closeBtn.dataset.closeTab!, 10);
      if (app.tabs.length > 1) {
        const closing = app.tabs[idx];
        if (closing.running) {
          const proceed = window.confirm("A task is still running in this tab. Close it? The agent will keep running until it finishes or you cancel it via /cancel before closing.");
          if (!proceed) return;
        }
        app.tabs.splice(idx, 1);
        if (app.active >= app.tabs.length) app.active = app.tabs.length - 1;
        renderAll();
      }
      return;
    }
    const tabEl = t.closest<HTMLElement>("[data-tab]");
    if (tabEl) {
      app.active = parseInt(tabEl.dataset.tab!, 10);
      renderAll();
    }
  });

  // Modal backdrop click (delegation on the modal).
  document.querySelector("#modal")!.addEventListener("click", (e) => {
    if (e.target === document.querySelector("#modal")) closeModal();
  });
  document.querySelector("#provider-modal")?.addEventListener("click", (e) => {
    if (e.target === document.querySelector("#provider-modal")) closeProviderModal();
  });
  document.querySelector("#model-modal")?.addEventListener("click", (e) => {
    if (e.target === document.querySelector("#model-modal")) closeModelModal();
  });
  document.querySelector("#provider-modal-close")?.addEventListener("click", () => closeProviderModal());
  document.querySelector("#model-modal-close")?.addEventListener("click", () => closeModelModal());
  document.querySelector("#diff-modal")?.addEventListener("click", (e) => {
    if (e.target === document.querySelector("#diff-modal")) closeDiffModal();
  });
  document.querySelector("#diff-modal-close")?.addEventListener("click", closeDiffModal);
  // Escape closes topmost modal (with dirty-check for forms).
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape") return;
    if (document.querySelector("#diff-modal")?.classList.contains("open")) { closeDiffModal(); return; }
    if (document.querySelector("#model-modal")?.classList.contains("open")) { closeModelModal(); return; }
    if (document.querySelector("#provider-modal")?.classList.contains("open")) { closeProviderModal(); return; }
    const settingsModal = document.querySelector("#modal");
    if (settingsModal && !settingsModal.classList.contains("hidden")) closeModal();
  });
  document.querySelector("#changes-list")?.addEventListener("click", (e) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>("[data-file-path]");
    if (row?.dataset.filePath) void openDiffViewer(row.dataset.filePath);
  });

  // Input.
  const input = document.querySelector<HTMLTextAreaElement>("#prompt-input")!;
  const sendPrompt = () => void send();
  document.querySelector("#send-btn")!.addEventListener("click", sendPrompt);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendPrompt();
    }
  });
  input.addEventListener("input", () => {
    input.style.height = "auto";
    input.style.height = Math.min(input.scrollHeight, 200) + "px";
  });

  // Mode toggle.
  document.querySelector("#mode-btn")!.addEventListener("click", () => {
    app.mode = app.mode === "build" ? "plan" : "build";
    renderHeader();
  });

  // Per-session role assignment panel (v0.17).
  document.querySelector("#model-btn")!.addEventListener("click", () => openRolePanel());

  // Version label.
  try {
    const v = await api.version();
    document.querySelector("#version-label")!.textContent = `v${v}`;
  } catch {
    /* ignore */
  }

  // v0.17: workspace initialization.
  await initWorkspace();
}

// ----- Workspace management (v0.17) -----

async function initWorkspace(): Promise<void> {
  // v0.17: migrate legacy Model 1/2/3 config into the provider registry (once).
  try { await api.migrateLegacyModels(); } catch { /* ignore */ }

  try {
    app.providers = await api.providersList();
  } catch { app.providers = []; }

  const recent = await api.workspaceList(10).catch(() => [] as WorkspaceDto[]);
  if (recent.length === 0) {
    showWorkspaceHome([]);
  } else {
    showWorkspaceHome(recent);
  }
  wireWorkspaceHome();
}

function showWorkspaceHome(recent: WorkspaceDto[]): void {
  const home = document.querySelector<HTMLDivElement>("#workspace-home")!;
  const sidebar = document.querySelector<HTMLDivElement>("#workspace-sidebar")!;
  home.style.display = "flex";
  sidebar.style.display = "none";
  updateMainMargins();
  void unwatchCurrentWorkspace();

  const subtitle = document.querySelector("#ws-home-subtitle")!;
  subtitle.textContent = recent.length > 0
    ? "Welcome back. Select a folder to continue."
    : "Select a folder to start working.";

  const list = document.querySelector<HTMLDivElement>("#ws-recent-list")!;
  if (recent.length === 0) {
    list.innerHTML = "";
    return;
  }
  list.innerHTML = recent.map((w) => `
    <button data-ws-open="${escapeAttr(w.id)}" data-ws-path="${escapeAttr(w.path)}"
      class="w-full text-left bg-app-surface border border-app-border rounded-lg px-4 py-3 hover:border-app-brand transition-colors">
      <div class="text-sm font-medium text-app-textPrimary">${escapeHtml(w.name)}</div>
      <div class="text-xs text-app-textSecondary font-mono truncate">${escapeHtml(w.path)}</div>
    </button>
  `).join("");
}

function wireWorkspaceHome(): void {
  document.querySelector("#ws-select-folder-btn")!.addEventListener("click", () => void pickFolder());
  document.querySelector("#ws-home-settings-link")?.addEventListener("click", () => openModal("settings"));
  document.querySelector("#ws-recent-list")!.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest<HTMLElement>("[data-ws-path]");
    if (btn) void openWorkspaceByPath(btn.dataset.wsPath!);
  });
  document.querySelector("#ws-switch-btn")?.addEventListener("click", () => void pickFolder());
  document.querySelector("#ws-new-session-btn")?.addEventListener("click", () => void createNewSession());
  document.querySelector("#ws-session-list")?.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest<HTMLElement>("[data-session-open]");
    if (btn) void openSessionById(btn.dataset.sessionOpen!);
  });
}

async function pickFolder(): Promise<void> {
  try {
    const selected = await api.pickFolder();
    if (selected) {
      await openWorkspaceByPath(selected);
    }
  } catch (e) {
    const msg = `Folder picker failed: ${String(e).slice(0,200)}`;
    const sub = document.querySelector("#ws-home-subtitle") as HTMLElement | null;
    if (sub) sub.textContent = msg;
    console.error(msg);
    try {
      const s = current();
      s.blocks.push({ id: newId(s), kind: "error", text: msg });
      renderAll();
    } catch {}
  }
}

async function openWorkspaceByPath(path: string): Promise<void> {
  try {
    const ws = await api.workspaceOpenFolder(path);
    app.workspace = ws;
    await loadWorkspaceSessions();
    showWorkspaceUi();
  } catch (e) {
    const msg = `Failed to open workspace: ${String(e).slice(0,200)}`;
    const sub = document.querySelector("#ws-home-subtitle") as HTMLElement | null;
    if (sub) { sub.textContent = msg; sub.classList.add("text-app-error"); }
    console.error(msg);
    try {
      const s = current();
      s.blocks.push({ id: newId(s), kind: "error", text: msg });
      renderAll();
    } catch {}
  }
}

async function loadWorkspaceSessions(): Promise<void> {
  if (!app.workspace) return;
  try {
    app.workspaceSessions = await api.workspaceSessions(app.workspace.id, 50);
  } catch { app.workspaceSessions = []; }
}

function showWorkspaceUi(): void {
  const home = document.querySelector<HTMLDivElement>("#workspace-home")!;
  const sidebar = document.querySelector<HTMLDivElement>("#workspace-sidebar")!;
  home.style.display = "none";
  sidebar.style.display = "flex";
  if (isSidebarCollapsed()) sidebar.classList.add("collapsed");
  else sidebar.classList.remove("collapsed");
  updateMainMargins();

  if (app.workspace) {
    document.querySelector("#ws-sidebar-name")!.textContent = app.workspace.name;
    document.querySelector("#ws-sidebar-path")!.textContent = app.workspace.path;
  }
  renderSessionList();
  void watchCurrentWorkspace();
}

function renderSessionList(): void {
  const list = document.querySelector<HTMLDivElement>("#ws-session-list")!;
  if (app.workspaceSessions.length === 0) {
    list.innerHTML = `<div class="text-xs text-app-textSecondary italic px-2 py-4 text-center">No sessions yet. Create one to start.</div>`;
    return;
  }
  list.innerHTML = app.workspaceSessions.map((s) => {
    const label = s.title || s.task || "Untitled session";
    const date = s.created_at.slice(0, 10);
    return `
      <button data-session-open="${escapeAttr(s.id)}"
        class="w-full text-left px-3 py-2 rounded hover:bg-app-hover transition-colors group">
        <div class="text-xs text-app-textPrimary truncate">${escapeHtml(label)}</div>
        <div class="text-[10px] text-app-textSecondary">${escapeHtml(date)}</div>
      </button>`;
  }).join("");
}

function renderChangesPanel(): void {
  const panel = document.querySelector<HTMLDivElement>("#changes-panel");
  const listEl = document.querySelector<HTMLDivElement>("#changes-list");
  const emptyEl = document.querySelector<HTMLDivElement>("#changes-empty");
  const summaryEl = document.querySelector<HTMLSpanElement>("#changes-summary");
  if (!panel || !listEl || !emptyEl || !summaryEl) return;
  const ch = app.workspaceChanges;
  if (!ch || ch.total_files === 0) {
    summaryEl.textContent = ch ? "No changes" : "No changes";
    listEl.innerHTML = "";
    emptyEl.style.display = "block";
    listEl.style.display = "none";
    return;
  }
  emptyEl.style.display = "none";
  listEl.style.display = "block";
  summaryEl.textContent = `${ch.total_files} files · +${ch.additions} -${ch.deletions}`;
  // Reuse existing diff viewer style: status pill + path + +/-
  listEl.innerHTML = ch.files.map((f) => {
    const statusClass = f.status === "M" ? "M" : f.status === "A" ? "A" : f.status === "D" ? "D" : f.status === "R" ? "R" : "U";
    const counts = (f.additions || f.deletions) ? `<span class="text-[10px] font-mono"><span class="text-green-400">+${f.additions}</span> <span class="text-red-400">-${f.deletions}</span></span>` : "";
    const renamed = f.renamed_from ? `<span class="text-[10px] text-app-textSecondary">→ ${escapeHtml(f.renamed_from)} →</span>` : "";
    return `<div class="changes-file" data-file-path="${escapeAttr(f.path)}" title="${escapeAttr(f.path)}">
      <div class="flex items-center space-x-2 min-w-0">
        <span class="status ${statusClass}">${escapeHtml(f.status)}</span>
        <span class="text-xs font-mono truncate text-app-textPrimary">${escapeHtml(f.path)}</span>
      </div>
      <div class="flex items-center space-x-2 shrink-0 ml-2">${renamed}${counts}</div>
    </div>`;
  }).join("");
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const lucide = (window as any).lucide; if (lucide?.createIcons) lucide.createIcons();
}

async function loadWorkspaceChanges(): Promise<void> {
  if (!app.workspace) return;
  try {
    const ch = await api.getWorkspaceChanges(app.workspace.id);
    // Only apply if still same workspace (session isolation)
    if (app.workspace && ch.workspace_id === app.workspace.id) {
      app.workspaceChanges = ch;
      renderChangesPanel();
      // Non-Git workspace: tell the user why counts are unavailable.
      const emptyEl = document.querySelector<HTMLDivElement>("#changes-empty");
      if (emptyEl && !ch.is_git && ch.total_files === 0) {
        emptyEl.textContent = "Not a git repository — only files changed during this session are listed.";
        emptyEl.style.display = "block";
      } else if (emptyEl) {
        emptyEl.textContent = "No changes";
      }
    }
  } catch (e) {
    console.warn("getWorkspaceChanges failed", e);
  }
}

async function watchCurrentWorkspace(): Promise<void> {
  if (!app.workspace) return;
  const wid = app.workspace.id;
  const prevWid = app.lastWatchedWorkspaceId;
  if (prevWid && prevWid !== wid) {
    try { await api.unwatchWorkspace(prevWid); } catch {}
  }
  app.lastWatchedWorkspaceId = wid;
  if (app.changesUnlisten) {
    try { app.changesUnlisten(); } catch {}
    app.changesUnlisten = null;
  }
  try {
    await api.watchWorkspace(wid);
  } catch (e) {
    console.warn("watchWorkspace failed", e);
    // Gracefully degrade: still try to load once
  }
  try {
    const unlisten = await events.onWorkspaceChanges((ch) => {
      try {
        if (!app.workspace || ch.workspace_id !== app.workspace.id) return;
        app.workspaceChanges = ch;
        renderChangesPanel();
      } catch (err) {
        console.error("workspace_changes handler failed", err);
      }
    });
    app.changesUnlisten = unlisten;
  } catch (e) {
    console.warn("onWorkspaceChanges listen failed", e);
  }
  await loadWorkspaceChanges();
}

async function unwatchCurrentWorkspace(): Promise<void> {
  const wid = app.lastWatchedWorkspaceId ?? app.workspace?.id;
  if (app.changesUnlisten) {
    try { app.changesUnlisten(); } catch {}
    app.changesUnlisten = null;
  }
  if (wid) {
    try { await api.unwatchWorkspace(wid); } catch {}
  }
  app.lastWatchedWorkspaceId = null;
  app.workspaceChanges = null;
  renderChangesPanel();
}

async function openDiffViewer(filePath: string): Promise<void> {
  if (!app.workspace) return;
  const modal = document.querySelector<HTMLDivElement>("#diff-modal")!;
  const title = document.querySelector<HTMLHeadingElement>("#diff-modal-title")!;
  const stats = document.querySelector<HTMLSpanElement>("#diff-modal-stats")!;
  const body = document.querySelector<HTMLDivElement>("#diff-modal-body")!;
  title.textContent = filePath;
  stats.textContent = "Loading…";
  body.innerHTML = `<div class="p-6 text-app-textSecondary text-sm">Loading diff…</div>`;
  modal.classList.add("open");
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const lucide = (window as any).lucide; if (lucide?.createIcons) lucide.createIcons();
  try {
    const diff = await api.getFileDiff(app.workspace.id, filePath);
    stats.textContent = `+${diff.additions} -${diff.deletions}`;
    if (!diff.diff || diff.diff.trim() === "") {
      body.innerHTML = `<div class="p-6 text-app-textSecondary text-sm">No diff available for ${escapeHtml(filePath)}.</div>`;
      return;
    }
    const lines = diff.diff.replace(/\r\n/g, "\n").split("\n");
    body.innerHTML = lines.map((line) => {
      const esc = escapeHtml(line);
      if (line.startsWith("+++") || line.startsWith("---")) {
        return `<div class="diff-line hunk">${esc}</div>`;
      } else if (line.startsWith("@@")) {
        return `<div class="diff-line hunk">${esc}</div>`;
      } else if (line.startsWith("+")) {
        return `<div class="diff-line add">${esc}</div>`;
      } else if (line.startsWith("-")) {
        return `<div class="diff-line del">${esc}</div>`;
      } else {
        return `<div class="diff-line ctx">${esc}</div>`;
      }
    }).join("");
  } catch (e) {
    stats.textContent = "error";
    body.innerHTML = `<div class="p-6 text-app-error text-sm">Failed to load diff: ${escapeHtml(String(e).slice(0,300))}</div>`;
  }
}

function closeDiffModal(): void {
  const modal = document.querySelector<HTMLDivElement>("#diff-modal")!;
  modal.classList.remove("open");
}

async function createNewSession(): Promise<void> {
  if (!app.workspace) return;
  try {
    const id = await api.workspaceCreateSession(app.workspace.id, undefined);
    await loadWorkspaceSessions();
    renderSessionList();
    const s = createSession("New Session");
    s.id = id;
    app.tabs.push(s);
    app.active = app.tabs.length - 1;
    app.roleAssignments = null;
    renderAll();
  } catch (e) {
    const msg = `Failed to create session: ${String(e).slice(0,200)}`;
    try {
      const s = current();
      s.blocks.push({ id: newId(s), kind: "error", text: msg });
      renderAll();
    } catch { console.error(msg); }
  }
}

async function openSessionById(sessionId: string): Promise<void> {
  const existing = app.tabs.findIndex((t) => t.id === sessionId);
  if (existing >= 0) {
    app.active = existing;
    await loadSessionRoles(sessionId);
    renderAll();
    return;
  }
  const meta = app.workspaceSessions.find((s) => s.id === sessionId);
  const title = meta?.title || meta?.task || "Session";
  const s = createSession(title);
  s.id = sessionId;
  app.tabs.push(s);
  app.active = app.tabs.length - 1;
  await loadSessionRoles(sessionId);
  if (app.workspace) void api.workspaceSetLastSession(app.workspace.id, sessionId).catch(() => {});
  renderAll();
  // Hydrate chat history from SQLite so past sessions aren't blank.
  try {
    const msgs = await api.getSessionMessages(sessionId);
    for (const m of msgs) {
      if (m.role === "user") {
        appendLine(s, m.content);
        // Mark the trailing user block visually by re-classifying is unnecessary; user blocks render right-aligned.
      } else if (m.role === "assistant") {
        appendLine(s, m.content);
      } else if (m.content && m.content.trim()) {
        appendLine(s, m.content);
      }
    }
    // Convert plain appended lines into role-tagged blocks: rebuild simply.
    if (msgs.length > 0) {
      s.blocks = [];
      s.nextId = 1;
      let lastRole = "";
      for (const m of msgs) {
        if (!m.content || !m.content.trim()) continue;
        const kind: BlockKind = m.role === "user" ? "user" : m.role === "tool" ? "tool" : "assistant";
        if (kind === "assistant" && lastRole === "assistant" && s.blocks.length > 0 && s.blocks[s.blocks.length-1].kind === "assistant") {
          const prev = s.blocks[s.blocks.length-1];
          prev.text += (prev.text ? "\n" : "") + m.content;
        } else {
          s.blocks.push({ id: newId(s), kind, text: m.content });
        }
        lastRole = kind;
      }
    }
    if (current() === s) renderAll();
  } catch (e) {
    console.warn("history hydration failed", e);
  }
}

async function loadSessionRoles(sessionId: string): Promise<void> {
  try {
    const json = await api.sessionGetRoles(sessionId);
    app.roleAssignments = json ? (JSON.parse(json) as RoleAssignmentsDto) : null;
  } catch {
    app.roleAssignments = null;
  }
}

// ----- Per-session role assignment panel (v0.17) -----

function allModelOptions(): Array<{ provider_id: string; model_id: string; label: string; vision: boolean }> {
  const out: Array<{ provider_id: string; model_id: string; label: string; vision: boolean }> = [];
  for (const p of app.providers) {
    for (const m of p.models) {
      out.push({
        provider_id: p.id,
        model_id: m.id,
        label: `${p.display_name || p.id} / ${m.display_name || m.id}`,
        vision: m.vision,
      });
    }
  }
  return out;
}

function bindingLabel(b: { provider_id: string; model_id: string } | null): string {
  if (!b) return "— not set —";
  const p = app.providers.find((x) => x.id === b.provider_id);
  const m = p?.models.find((x) => x.id === b.model_id);
  return `${p?.display_name || b.provider_id} / ${m?.display_name || b.model_id}`;
}

function openRolePanel(): void {
  const s = current();
  const opts = allModelOptions();
  const ra = app.roleAssignments ?? { executor: null, controller: null, reviewer: null };
  const bodyPre = document.querySelector<HTMLDivElement>("#modal-body")!;
  const modalPre = document.querySelector<HTMLDivElement>("#modal")!;
  if (opts.length === 0) {
    document.querySelector("#modal-title")!.textContent = "LLM Configuration";
    modalPre.classList.remove("hidden");
    modalPre.classList.add("flex");
    const hasProviders = app.providers.length > 0;
    bodyPre.innerHTML = `
      <div class="space-y-4 max-w-xl">
        <p class="text-sm text-app-textSecondary">${hasProviders ? "Providers exist but no models are configured. Add a model to one of your providers in Settings to enable chat." : "No providers or models configured. Add a provider and a model in Settings to enable chat."}</p>
        <button id="role-open-settings" type="button" class="text-xs px-4 py-1.5 bg-app-brand text-app-bg rounded font-semibold">Open Settings</button>
      </div>`;
    bodyPre.querySelector<HTMLButtonElement>("#role-open-settings")?.addEventListener("click", () => {
      closeModal();
      openModal("settings");
    });
    return;
  }

  const selectHtml = (role: "executor" | "controller" | "reviewer", requireVision: boolean) => {
    const cur = ra[role];
    const options = opts
      .map((o) => {
        const disabled = requireVision && !o.vision;
        const selected = cur && cur.provider_id === o.provider_id && cur.model_id === o.model_id ? "selected" : "";
        return `<option value="${escapeAttr(o.provider_id)}|${escapeAttr(o.model_id)}" ${selected} ${disabled ? "disabled" : ""}>${escapeHtml(o.label)}${disabled ? " (vision unavailable)" : ""}</option>`;
      })
      .join("");
    return `<select data-role-select="${role}" class="w-full bg-app-bg border border-app-border rounded px-2 py-1.5 text-xs font-mono text-app-textPrimary">
      <option value="">— not set —</option>
      ${options}
    </select>`;
  };

  const body = document.querySelector<HTMLDivElement>("#modal-body")!;
  const modal = document.querySelector<HTMLDivElement>("#modal")!;
  document.querySelector("#modal-title")!.textContent = "LLM Configuration";
  modal.classList.remove("hidden");
  modal.classList.add("flex");
  body.innerHTML = `
    <div class="space-y-4 max-w-xl">
      <p class="text-xs text-app-textSecondary">Choose which model performs each AETHER role for this session. Changes apply to future turns only.</p>
      <div>
        <label class="text-xs uppercase tracking-wide text-app-textSecondary block mb-1">LLM 1 — Big Executor</label>
        ${selectHtml("executor", false)}
      </div>
      <div>
        <label class="text-xs uppercase tracking-wide text-app-textSecondary block mb-1">LLM 2 — Small Controller</label>
        ${selectHtml("controller", false)}
      </div>
      <div>
        <label class="text-xs uppercase tracking-wide text-app-textSecondary block mb-1">LLM 3 — Visual Reviewer (optional, requires vision)</label>
        ${selectHtml("reviewer", true)}
      </div>
      <div class="flex items-center space-x-3 pt-3 border-t border-app-border">
        <button id="role-save-btn" type="button" class="bg-app-brand text-app-bg font-semibold px-4 py-1.5 rounded text-sm hover:opacity-90">Save</button>
        <span id="role-save-status" class="text-xs text-app-textSecondary"></span>
      </div>
    </div>`;

  body.querySelector<HTMLButtonElement>("#role-save-btn")?.addEventListener("click", async () => {
    const status = body.querySelector<HTMLSpanElement>("#role-save-status")!;
    const read = (role: "executor" | "controller" | "reviewer") => {
      const v = body.querySelector<HTMLSelectElement>(`[data-role-select="${role}"]`)?.value ?? "";
      if (!v) return null;
      const [provider_id, model_id] = v.split("|");
      return { provider_id, model_id };
    };
    const next: RoleAssignmentsDto = {
      executor: read("executor"),
      controller: read("controller"),
      reviewer: read("reviewer"),
    };
    if (!next.executor || !next.controller) {
      status.textContent = "LLM 1 (Executor) and LLM 2 (Controller) are required.";
      return;
    }
    try {
      // Welcome tab uses a client-generated UUID that may not exist in sessions.db yet.
      // Create the session row lazily so role-save doesn't fail with a raw DB error.
      const known = app.workspaceSessions.some(ws => ws.id === s.id);
      if (!known && app.workspace) {
        try {
          await api.workspaceSetLastSession(app.workspace.id, s.id);
        } catch { /* best-effort; set_roles will surface a real error if the row truly can't be created */ }
      }
      await api.sessionSetRoles(s.id, JSON.stringify(next));
      app.roleAssignments = next;
      status.textContent = "Saved.";
      renderHeader();
    } catch (e) {
      status.textContent = `Save failed: ${redactSecrets(String(e))}`;
    }
  });
}

boot();
