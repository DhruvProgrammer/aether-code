import {
  api,
  events,
  type TaskExit,
  type TaskOutput,
  type DesktopConfig,
  type ModelBlock,
  type AppearanceConfig,
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
  modelKey: "controller",
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
  document.querySelector("#model-label")!.textContent = app.modelKey;
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

let activeUnlistens: (() => void)[] = [];

async function send() {
  const input = document.querySelector<HTMLTextAreaElement>("#prompt-input")!;
  const text = input.value.trim();
  if (!text) return;

  const s = current();
  s.title = text.slice(0, 40);
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
        "/new          Start a new tab",
        "/locate       Show where the aether CLI was found",
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
  if (text === "/cancel") {
    if (s.running) {
      await api.cancelTask(s.id);
      s.blocks.push({ id: newId(s), kind: "info", text: "Cancel requested." });
      renderAll();
    }
    return;
  }
  if (text === "/locate") {
    try {
      const r = await api.locateCli();
      const found = r.found ? `Found: ${r.found}` : "Not found.";
      s.blocks.push({ id: newId(s), kind: "info", text: `${found}\nSearched:\n  ${r.searched.join("\n  ")}` });
    } catch (e) {
      s.blocks.push({ id: newId(s), kind: "error", text: `locate failed: ${String(e)}` });
    }
    renderAll();
    return;
  }

  // Real task.
  s.running = true;
  renderAll();
  attachListeners(s);

  try {
    const handle = await api.runTask(text, app.mode === "plan");
    s.id = handle.session_id;
  } catch (e) {
    s.running = false;
    s.blocks.push({ id: newId(s), kind: "error", text: `Failed to start: ${String(e)}` });
    renderAll();
    detachListeners();
  }
}

function attachListeners(s: Session) {
  detachListeners();
  events.onTaskOutput((o: TaskOutput) => {
    if (o.session_id !== s.id) return;
    appendLine(s, o.line);
    renderStream();
  }).then((u) => activeUnlistens.push(u));
  events.onTaskExit((e: TaskExit) => {
    if (e.session_id !== s.id) return;
    s.running = false;
    if (!e.success) {
      s.blocks.push({ id: newId(s), kind: "error", text: `Exit code: ${e.code ?? "?"}` });
    }
    renderAll();
    detachListeners();
  }).then((u) => activeUnlistens.push(u));
}

function detachListeners() {
  for (const u of activeUnlistens) try { u(); } catch {}
  activeUnlistens = [];
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
//                     preview, opacity slider) and the required-resolution
//                     gate. The bundled default ships with the application;
//                     user-supplied images must be exactly 1920x1080 px and
//                     are validated server-side (spec §13-§14).
//
// Validation rules (spec §7):
//   * Model 1 — every field required.
//   * Model 2/3 — completely empty is VALID; partially configured is INVALID
//     and the UI shows which field is missing.

interface SlotState {
  model_id: string;
  provider: string;
  base_url: string;
  api_key: string;
}

function emptySlot(): SlotState {
  return { model_id: "", provider: "", base_url: "", api_key: "" };
}

function buildSlots(cfg: DesktopConfig): { m1: SlotState; m2: SlotState; m3: SlotState } {
  const m1key = (cfg.agent.model1 || cfg.agent.executor_model || "executor").trim() || "executor";
  const m2key = (cfg.agent.model2 ?? cfg.agent.controller_model ?? "").trim();
  const m3key = (cfg.agent.model3 ?? cfg.agent.reviewer_model ?? "").trim();

  const lookup = (key: string): ModelBlock | null => {
    if (!key) return null;
    const m = cfg.models[key];
    if (!m) return null;
    return m;
  };

  const fromKey = (key: string): SlotState => {
    const m = lookup(key);
    if (!m) return { ...emptySlot(), model_id: key };
    return {
      model_id: key,
      provider: m.provider ?? "",
      base_url: m.base_url ?? "",
      api_key: m.api_key_env ?? "",
    };
  };

  return {
    m1: fromKey(m1key),
    m2: m2key ? fromKey(m2key) : emptySlot(),
    m3: m3key ? fromKey(m3key) : emptySlot(),
  };
}

function renderSlotCard(opts: {
  slot: 1 | 2 | 3;
  role: string;
  required: boolean;
  state: SlotState;
}): string {
  const { slot, role, required, state } = opts;
  const sfx = String(slot);
  const pillCls = required ? "role-pill" : "role-pill opt";
  return `
    <div class="model-card" data-slot-card="${sfx}">
      <div class="flex items-center justify-between">
        <div>
          <div class="slot-title">MODEL ${sfx}</div>
          <div class="role-desc">${escapeHtml(role)}</div>
        </div>
        <span class="${pillCls}">${required ? "Required" : "Optional"}</span>
      </div>
      <div class="field">
        <label>Model ID</label>
        <input data-slot="${sfx}" data-field="model_id" value="${escapeAttr(state.model_id)}" placeholder="e.g. qwen-coder" />
      </div>
      <div class="field">
        <label>Provider</label>
        <input data-slot="${sfx}" data-field="provider" value="${escapeAttr(state.provider)}" placeholder="openai_compatible" />
      </div>
      <div class="field">
        <label>Base URL</label>
        <input data-slot="${sfx}" data-field="base_url" value="${escapeAttr(state.base_url)}" placeholder="https://api.example.com/v1" />
      </div>
      <div class="field">
        <label>API Key</label>
        <input data-slot="${sfx}" data-field="api_key" value="${escapeAttr(state.api_key)}" placeholder="OPENAI_API_KEY" />
      </div>
      <div class="validation-err" data-slot-err="${sfx}" style="display:none"></div>
    </div>`;
}

function renderProviderRow(key: string, m: ModelBlock | null): string {
  const v = m ?? { provider: "", base_url: "", model: "", api_key_env: "OPENAI_API_KEY" };
  return `<div class="provider-row" data-prov-row="${escapeAttr(key)}">
    <input data-prov="key" value="${escapeAttr(key)}" placeholder="provider-id" />
    <input data-prov="provider" value="${escapeAttr(v.provider)}" placeholder="openai_compatible" />
    <input data-prov="url" value="${escapeAttr(v.base_url)}" placeholder="https://api.example.com/v1" />
    <input data-prov="model" value="${escapeAttr(v.model)}" placeholder="model-id" />
    <input data-prov="env" value="${escapeAttr(v.api_key_env)}" placeholder="OPENAI_API_KEY" />
    <button type="button" class="text-app-textSecondary hover:text-app-error" data-prov-del title="Remove">
      <i class="w-4 h-4" data-lucide="trash-2"></i>
    </button>
  </div>`;
}

async function renderSettings(cfg: DesktopConfig, path: string): Promise<string> {
  const slots = buildSlots(cfg);
  const providerEntries = Object.entries(cfg.models ?? {});

  // Required-resolution label is server-authoritative.
  let requiredRes = "1920 x 1080 px";
  try { requiredRes = await api.requiredBackgroundResolution(); } catch { /* default */ }

  const appearance: AppearanceConfig = cfg.appearance ?? {
    background_enabled: true,
    background_opacity: 60,
    background_image: null,
  };

  return `
    <div class="space-y-1">
      <p class="text-app-textSecondary text-xs">Saved to <code class="font-mono">${escapeHtml(path)}</code></p>

      <!-- ───── AI / MODELS ───── -->
      <section class="settings-section">
        <h3>AI / Models</h3>
        <p class="text-xs text-app-textSecondary mb-3">Three model slots, provider-independent. Model 1 is required; Model 2 and Model 3 are optional.</p>
        <div class="settings-grid" id="slots-grid">
          ${renderSlotCard({ slot: 1, role: "Big Executor", required: true,  state: slots.m1 })}
          ${renderSlotCard({ slot: 2, role: "Controller",     required: false, state: slots.m2 })}
          ${renderSlotCard({ slot: 3, role: "Visual Reviewer", required: false, state: slots.m3 })}
        </div>
      </section>

      <!-- ───── PROVIDERS ───── -->
      <section class="settings-section">
        <h3>Providers</h3>
        <p class="text-xs text-app-textSecondary mb-3">OpenAI-compatible providers. Each contains only the fields AETHER actually uses.</p>
        <div class="provider-row header">
          <div>Provider ID</div><div>Type</div><div>Base URL</div><div>Models</div><div>API Key</div><div></div>
        </div>
        <div id="providers-body" class="space-y-2 mt-2">
          ${providerEntries.map(([k, m]) => renderProviderRow(k, m)).join("")}
        </div>
        <button id="add-provider" type="button" class="mt-3 text-xs text-app-textSecondary hover:text-app-brand inline-flex items-center">
          <i class="w-3.5 h-3.5 mr-1" data-lucide="plus"></i> Add provider
        </button>
      </section>

      <!-- ───── APPEARANCE ───── -->
      <section class="settings-section">
        <h3>Appearance</h3>
        <p class="text-xs text-app-textSecondary mb-3">Native background image. Ships with a bundled default; user-supplied images must be exactly ${escapeHtml(requiredRes)}.</p>

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

function readSlot(body: HTMLElement, slot: 1 | 2 | 3): SlotState {
  const sfx = String(slot);
  const get = (f: string): string =>
    body.querySelector<HTMLInputElement>(`input[data-slot="${sfx}"][data-field="${f}"]`)?.value.trim() ?? "";
  return { model_id: get("model_id"), provider: get("provider"), base_url: get("base_url"), api_key: get("api_key") };
}

function validateSlots(slots: { m1: SlotState; m2: SlotState; m3: SlotState }): { ok: boolean; errors: Record<number, string> } {
  const errors: Record<number, string> = {};

  // Model 1 — required, every field must be present.
  const missing1: string[] = [];
  if (!slots.m1.model_id)  missing1.push("Model ID");
  if (!slots.m1.provider)  missing1.push("Provider");
  if (!slots.m1.base_url)  missing1.push("Base URL");
  if (!slots.m1.api_key)   missing1.push("API Key");
  if (missing1.length > 0) errors[1] = `Model 1 (Big Executor) is required. Missing: ${missing1.join(", ")}`;

  // Model 2 — empty is OK; partial is INVALID.
  const m2keys = ["model_id", "provider", "base_url", "api_key"] as const;
  const m2nonEmpty = m2keys.filter((k) => slots.m2[k].length > 0).length;
  if (m2nonEmpty > 0 && m2nonEmpty < m2keys.length) {
    const missing2 = m2keys.filter((k) => !slots.m2[k]);
    errors[2] = `Model 2 (Controller) is partially configured. Either fill all fields or leave it empty. Missing: ${missing2.join(", ")}`;
  }

  // Model 3 — empty is OK; partial is INVALID.
  const m3nonEmpty = m2keys.filter((k) => slots.m3[k].length > 0).length;
  if (m3nonEmpty > 0 && m3nonEmpty < m2keys.length) {
    const missing3 = m2keys.filter((k) => !slots.m3[k]);
    errors[3] = `Model 3 (Visual Reviewer) is partially configured. Either fill all fields or leave it empty. Missing: ${missing3.join(", ")}`;
  }

  return { ok: Object.keys(errors).length === 0, errors };
}

function showSlotErrors(body: HTMLElement, errors: Record<number, string>): void {
  for (let slot = 1; slot <= 3; slot++) {
    const el = body.querySelector<HTMLDivElement>(`[data-slot-err="${slot}"]`);
    if (!el) continue;
    const msg = errors[slot];
    if (msg) {
      el.textContent = msg;
      el.style.display = "block";
    } else {
      el.textContent = "";
      el.style.display = "none";
    }
  }
}

function collectProviders(body: HTMLElement): Record<string, ModelBlock> {
  const out: Record<string, ModelBlock> = {};
  body.querySelectorAll<HTMLDivElement>("#providers-body [data-prov-row]").forEach((row) => {
    const get = (f: string): string =>
      row.querySelector<HTMLInputElement>(`input[data-prov="${f}"]`)?.value.trim() ?? "";
    const key = get("key");
    if (!key) return;
    out[key] = {
      provider: get("provider"),
      base_url: get("url"),
      model: get("model"),
      api_key_env: get("env") || "OPENAI_API_KEY",
    };
  });
  return out;
}

function collectAppearance(body: HTMLElement, current: AppearanceConfig): AppearanceConfig {
  const enabled = body.querySelector<HTMLDivElement>("#bg-toggle")?.classList.contains("on") ?? current.background_enabled;
  const opacity = parseInt(body.querySelector<HTMLInputElement>("#bg-opacity")?.value ?? String(current.background_opacity), 10);
  return {
    background_enabled: enabled,
    background_opacity: isNaN(opacity) ? current.background_opacity : Math.max(0, Math.min(100, opacity)),
    background_image: current.background_image,
  };
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

  // ----- Provider add/remove -----
  body.querySelector<HTMLButtonElement>("#add-provider")?.addEventListener("click", () => {
    const unique = `provider${Object.keys(originalCfg.models).length + 1}`;
    const wrap = document.createElement("div");
    wrap.innerHTML = renderProviderRow(unique, null);
    const row = wrap.firstElementChild as HTMLDivElement;
    row.querySelector<HTMLButtonElement>("[data-prov-del]")?.addEventListener("click", () => row.remove());
    body.querySelector("#providers-body")!.appendChild(row);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const lucide = (window as any).lucide;
    if (lucide?.createIcons) lucide.createIcons();
  });
  body.querySelectorAll<HTMLButtonElement>("#providers-body [data-prov-del]").forEach((b) =>
    b.addEventListener("click", () => b.closest("[data-prov-row]")?.remove()));

  // ----- Live opacity preview -----
  const opacitySlider = body.querySelector<HTMLInputElement>("#bg-opacity");
  const opacityLabel = body.querySelector<HTMLSpanElement>("#bg-opacity-label");
  opacitySlider?.addEventListener("input", () => {
    if (opacityLabel) opacityLabel.textContent = `${opacitySlider.value}%`;
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

    const slots = { m1: readSlot(body, 1), m2: readSlot(body, 2), m3: readSlot(body, 3) };
    const validation = validateSlots(slots);
    showSlotErrors(body, validation.errors);
    if (!validation.ok) {
      status.textContent = "Fix the errors above before saving.";
      return;
    }

    const providers = collectProviders(body);
    // Make sure every slot's referenced provider exists in the map.
    for (const s of [slots.m1, slots.m2, slots.m3]) {
      if (s.model_id && !providers[s.model_id]) {
        // Promote the inline slot into a provider entry so the runtime can resolve it.
        providers[s.model_id] = {
          provider: s.provider,
          base_url: s.base_url,
          model: s.model_id,
          api_key_env: s.api_key || "OPENAI_API_KEY",
        };
      }
    }

    const appearance = collectAppearance(body, originalCfg.appearance);

    try {
      const path = await api.writeConfig({
        agent: {
          model1: slots.m1.model_id,
          model2: slots.m2.model_id,
          model3: slots.m3.model_id || null,
          // Legacy mirrors so older aether CLI builds keep working.
          executor_model: slots.m1.model_id,
          controller_model: slots.m2.model_id || "controller",
          reviewer_model: slots.m3.model_id || null,
        },
        models: providers,
        frontend: originalCfg.frontend,
        appearance,
      });
      status.textContent = `Saved to ${path}`;
      // Apply background live (no reload).
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
  applyBackgroundLayer(enabled, isNaN(opacity) ? 60 : opacity);
}

function applyBackgroundLayer(enabled: boolean, opacityPct: number): void {
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
  app.tabs.push(createSession("Welcome"));
  renderAll();

  // Apply background on launch (spec §16): we don't wait for the Settings modal.
  try {
    const r = await api.readConfig();
    const appearance = r.config.appearance ?? {
      background_enabled: true,
      background_opacity: 60,
      background_image: null,
    };
    try {
      const payload = await api.getBackground();
      if (payload.data_base64) {
        cachedBackground = { data: payload.data_base64, contentType: payload.content_type };
      }
    } catch { /* keep null; layer renders solid */ }
    applyBackgroundLayer(appearance.background_enabled, appearance.background_opacity);
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

  // Model picker (reads from config; surfaces the three model slots).
  try {
    const r = await api.readConfig();
    const slots = [r.config.agent.model1, r.config.agent.model2, r.config.agent.model3].filter(
      (k): k is string => !!k,
    );
    if (slots.length > 0) app.modelKey = slots[0];
    const modelBtn = document.querySelector("#model-btn")!;
    modelBtn.addEventListener("click", () => {
      if (slots.length === 0) return;
      const i = slots.indexOf(app.modelKey);
      app.modelKey = slots[(i + 1) % slots.length];
      renderHeader();
    });
  } catch {
    /* ignore */
  }

  // Version label.
  try {
    const v = await api.version();
    document.querySelector("#version-label")!.textContent = `v${v}`;
  } catch {
    /* ignore */
  }
}

boot();
