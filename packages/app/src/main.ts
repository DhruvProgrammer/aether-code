import { api, events, type TaskExit, type TaskOutput } from "./api";

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
        <i class="w-3.5 h-3.5 absolute right-2 text-app-textSecondary group-hover:text-app-error" data-lucide="x" data-close-tab="${i}"></i>
      </div>`;
    })
    .join("");
  el.querySelectorAll<HTMLDivElement>("[data-tab]").forEach((d) => {
    d.addEventListener("click", (e) => {
      const t = e.target as HTMLElement;
      if (t.dataset.closeTab != null) {
        const idx = parseInt(t.dataset.closeTab, 10);
        if (app.tabs.length > 1) {
          app.tabs.splice(idx, 1);
          if (app.active >= app.tabs.length) app.active = app.tabs.length - 1;
          renderAll();
        }
        return;
      }
      app.active = parseInt(d.dataset.tab!, 10);
      renderAll();
    });
  });
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
      body.innerHTML = renderSettings(r.config, r.path);
      wireSettings(body);
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

function renderSettings(cfg: import("./api").DesktopConfig, path: string): string {
  const m = cfg.models ?? {};
  const rows = Object.entries(m).map(([k, v]) => `
    <tr class="border-b border-app-border">
      <td class="py-2 pr-3"><input class="w-full bg-transparent border border-app-border rounded px-2 py-1 text-sm font-mono" data-mk="key" value="${escapeAttr(k)}" /></td>
      <td class="py-2 pr-3"><input class="w-full bg-transparent border border-app-border rounded px-2 py-1 text-sm font-mono" data-mk="provider" value="${escapeAttr(v.provider)}" /></td>
      <td class="py-2 pr-3"><input class="w-full bg-transparent border border-app-border rounded px-2 py-1 text-sm font-mono" data-mk="url" value="${escapeAttr(v.base_url)}" /></td>
      <td class="py-2 pr-3"><input class="w-full bg-transparent border border-app-border rounded px-2 py-1 text-sm font-mono" data-mk="model" value="${escapeAttr(v.model)}" /></td>
      <td class="py-2 pr-3"><input class="w-full bg-transparent border border-app-border rounded px-2 py-1 text-sm font-mono" data-mk="env" value="${escapeAttr(v.api_key_env)}" /></td>
      <td class="py-2"><button class="text-app-error hover:text-app-error/70" data-mk="del">×</button></td>
    </tr>`).join("");

  return `
    <div class="space-y-4">
      <p class="text-app-textSecondary text-xs">Saved to <code class="font-mono">${escapeHtml(path)}</code></p>

      <div class="space-y-2">
        <label class="block text-xs uppercase tracking-wide text-app-textSecondary">API key (env: OPENAI_API_KEY)</label>
        <input id="api-key" type="password" class="w-full bg-app-bg border border-app-border rounded px-3 py-2 text-sm font-mono" placeholder="sk-…" />
        <p class="text-xs text-app-textSecondary">The aether CLI reads this from your shell environment; this field is for reference only.</p>
      </div>

      <div class="space-y-2">
        <label class="block text-xs uppercase tracking-wide text-app-textSecondary">Models</label>
        <table class="w-full text-sm">
          <thead><tr class="text-app-textSecondary text-xs uppercase">
            <th class="py-1 text-left">Key</th><th class="py-1 text-left">Provider</th><th class="py-1 text-left">Base URL</th><th class="py-1 text-left">Model</th><th class="py-1 text-left">API key env</th><th></th>
          </tr></thead>
          <tbody id="models-body">${rows}</tbody>
        </table>
        <button id="add-model" class="text-xs text-app-textSecondary hover:text-app-brand">+ Add model</button>
      </div>

      <div class="grid grid-cols-2 gap-3">
        <div>
          <label class="block text-xs uppercase tracking-wide text-app-textSecondary mb-1">Controller</label>
          <input id="ctrl" class="w-full bg-app-bg border border-app-border rounded px-3 py-2 text-sm font-mono" value="${escapeAttr(cfg.agent.controller_model)}" />
        </div>
        <div>
          <label class="block text-xs uppercase tracking-wide text-app-textSecondary mb-1">Executor</label>
          <input id="exec" class="w-full bg-app-bg border border-app-border rounded px-3 py-2 text-sm font-mono" value="${escapeAttr(cfg.agent.executor_model)}" />
        </div>
        <div>
          <label class="block text-xs uppercase tracking-wide text-app-textSecondary mb-1">Reviewer (optional)</label>
          <input id="rev" class="w-full bg-app-bg border border-app-border rounded px-3 py-2 text-sm font-mono" value="${escapeAttr(cfg.agent.reviewer_model ?? "")}" />
        </div>
      </div>

      <div class="flex items-center space-x-3 pt-3 border-t border-app-border">
        <button id="save-btn" class="bg-app-brand text-app-bg font-semibold px-4 py-1.5 rounded text-sm hover:opacity-90">Save</button>
        <span id="save-status" class="text-xs text-app-textSecondary"></span>
      </div>
    </div>`;
}

function wireSettings(body: HTMLElement) {
  body.querySelector<HTMLButtonElement>("#add-model")?.addEventListener("click", () => {
    const tr = document.createElement("tr");
    tr.className = "border-b border-app-border";
    tr.innerHTML = `
      <td class="py-2 pr-3"><input class="w-full bg-transparent border border-app-border rounded px-2 py-1 text-sm font-mono" data-mk="key" value="newmodel" /></td>
      <td class="py-2 pr-3"><input class="w-full bg-transparent border border-app-border rounded px-2 py-1 text-sm font-mono" data-mk="provider" value="openai_compatible" /></td>
      <td class="py-2 pr-3"><input class="w-full bg-transparent border border-app-border rounded px-2 py-1 text-sm font-mono" data-mk="url" value="https://api.openai.com/v1" /></td>
      <td class="py-2 pr-3"><input class="w-full bg-transparent border border-app-border rounded px-2 py-1 text-sm font-mono" data-mk="model" value="gpt-4o-mini" /></td>
      <td class="py-2 pr-3"><input class="w-full bg-transparent border border-app-border rounded px-2 py-1 text-sm font-mono" data-mk="env" value="OPENAI_API_KEY" /></td>
      <td class="py-2"><button class="text-app-error hover:text-app-error/70" data-mk="del">×</button></td>`;
    tr.querySelector("[data-mk=del]")!.addEventListener("click", () => tr.remove());
    body.querySelector("#models-body")!.appendChild(tr);
  });

  body.querySelectorAll<HTMLButtonElement>("[data-mk=del]").forEach((b) =>
    b.addEventListener("click", () => b.closest("tr")?.remove()));

  body.querySelector<HTMLButtonElement>("#save-btn")?.addEventListener("click", async () => {
    const status = body.querySelector<HTMLSpanElement>("#save-status")!;
    status.textContent = "Saving…";
    const models: Record<string, import("./api").ModelBlock> = {};
    body.querySelectorAll<HTMLTableRowElement>("#models-body tr").forEach((tr) => {
      const k = (tr.querySelector<HTMLInputElement>("[data-mk=key]")!.value || "").trim();
      if (!k) return;
      models[k] = {
        provider: tr.querySelector<HTMLInputElement>("[data-mk=provider]")!.value.trim(),
        base_url: tr.querySelector<HTMLInputElement>("[data-mk=url]")!.value.trim(),
        model: tr.querySelector<HTMLInputElement>("[data-mk=model]")!.value.trim(),
        api_key_env: tr.querySelector<HTMLInputElement>("[data-mk=env]")!.value.trim() || "OPENAI_API_KEY",
        extra_body: null,
      };
    });
    try {
      const path = await api.writeConfig({
        agent: {
          controller_model: body.querySelector<HTMLInputElement>("#ctrl")!.value.trim() || "controller",
          executor_model: body.querySelector<HTMLInputElement>("#exec")!.value.trim() || "executor",
          reviewer_model: body.querySelector<HTMLInputElement>("#rev")!.value.trim() || null,
        },
        models,
        frontend: { capture_command: null, preview_command: null, max_visual_iterations: 5, force: false },
      });
      status.textContent = `Saved to ${path}`;
    } catch (e) {
      status.textContent = `Save failed: ${String(e)}`;
    }
  });
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

  // Top-bar buttons.
  document.querySelector("#new-tab-btn")!.addEventListener("click", newTab);
  document.querySelector("#open-settings-btn")!.addEventListener("click", () => openModal("settings"));
  document.querySelector("#open-history-btn")!.addEventListener("click", () => openModal("history"));
  document.querySelector("#modal-close")!.addEventListener("click", closeModal);
  document.querySelector("#modal")!.addEventListener("click", (e) => {
    if (e.target === document.querySelector("#modal")) closeModal();
  });

  // Input.
  const input = document.querySelector<HTMLTextAreaElement>("#prompt-input")!;
  const send = () => void sendPrompt();
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

  // Model picker (reads from config).
  try {
    const r = await api.readConfig();
    const keys = Object.keys(r.config.models);
    if (keys.length > 0) app.modelKey = keys[0];
    const modelBtn = document.querySelector("#model-btn")!;
    modelBtn.addEventListener("click", () => {
      if (keys.length === 0) return;
      const i = keys.indexOf(app.modelKey);
      app.modelKey = keys[(i + 1) % keys.length];
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
