import { api, type DesktopConfig, type ModelBlock } from "../api";

const root = document.querySelector<HTMLElement>("#view-settings")!;

let baseConfig: DesktopConfig | null = null;

export async function mountSettings() {
  const resp = await api.readConfig();
  baseConfig = resp.config;
  render(resp.config, resp.path);
}

export function unmountSettings() {}

function render(config: DesktopConfig, path: string) {
  root.innerHTML = `
    <div class="card">
      <h2>Settings</h2>
      <p class="muted">Saved to <code>${escapeHtml(path)}</code></p>

      <div class="row">
        <label>OpenAI API key (env: <code>OPENAI_API_KEY</code>)</label>
        <div class="row">
          <input type="password" id="api-key" placeholder="sk-…" />
          <button id="api-key-check" class="ghost">Check</button>
        </div>
        <p class="muted" id="api-key-status"></p>
      </div>

      <h3>Models</h3>
      <table id="models-table">
        <thead>
          <tr>
            <th>Key</th>
            <th>Provider</th>
            <th>Base URL</th>
            <th>Model</th>
            <th>API key env</th>
            <th></th>
          </tr>
        </thead>
        <tbody id="models-body"></tbody>
      </table>
      <button id="add-model" class="ghost">+ Add model</button>

      <h3>Agent routing</h3>
      <div class="row">
        <label>Controller model key</label>
        <input id="ctrl-model" value="${escapeAttr(config.agent.controller_model)}" />
      </div>
      <div class="row">
        <label>Executor model key</label>
        <input id="exec-model" value="${escapeAttr(config.agent.executor_model)}" />
      </div>
      <div class="row">
        <label>Reviewer model key (optional)</label>
        <input id="rev-model" value="${escapeAttr(config.agent.reviewer_model ?? "")}" />
      </div>

      <h3>Frontend (visual review)</h3>
      <div class="row">
        <label>Capture command (use <code>{cwd}</code> / <code>{out}</code>)</label>
        <input id="capture-cmd" value="${escapeAttr(config.frontend.capture_command ?? "")}" placeholder="npm --prefix {cwd} run build && …" />
      </div>
      <div class="row">
        <label>Preview command</label>
        <input id="preview-cmd" value="${escapeAttr(config.frontend.preview_command ?? "")}" placeholder="npm --prefix {cwd} run dev" />
      </div>
      <div class="row">
        <label>Max visual iterations</label>
        <input id="max-visual" type="number" value="${config.frontend.max_visual_iterations}" min="0" />
      </div>
      <div class="row inline">
        <input type="checkbox" id="force-visual" ${config.frontend.force ? "checked" : ""} />
        <label for="force-visual">Force visual review for every task</label>
      </div>

      <div class="actions">
        <button id="save-btn" class="primary">Save</button>
        <span id="save-status" class="muted"></span>
      </div>
    </div>
  `;

  const body = root.querySelector<HTMLTableSectionElement>("#models-body")!;
  for (const [k, m] of Object.entries(config.models)) {
    body.appendChild(modelRow(k, m));
  }

  root.querySelector<HTMLButtonElement>("#add-model")!.addEventListener("click", () => {
    const key = prompt("Model key (e.g. controller, executor, fast):");
    if (!key) return;
    body.appendChild(
      modelRow(key, {
        provider: "openai_compatible",
        base_url: "https://api.openai.com/v1",
        model: "gpt-4o-mini",
        api_key_env: "OPENAI_API_KEY",
        extra_body: null,
      }),
    );
  });

  root.querySelector<HTMLButtonElement>("#api-key-check")!.addEventListener("click", async () => {
    const status = root.querySelector<HTMLParagraphElement>("#api-key-status")!;
    const val = (root.querySelector<HTMLInputElement>("#api-key")!.value || "").trim();
    status.textContent = val
      ? "API key set in this field (saving the field does not export it to env — set OPENAI_API_KEY in your shell for the runtime to pick it up)."
      : "Empty.";
  });

  root.querySelector<HTMLButtonElement>("#save-btn")!.addEventListener("click", save);
}

function modelRow(key: string, m: ModelBlock): HTMLTableRowElement {
  const tr = document.createElement("tr");
  tr.innerHTML = `
    <td><input class="m-key" value="${escapeAttr(key)}" /></td>
    <td><input class="m-provider" value="${escapeAttr(m.provider)}" /></td>
    <td><input class="m-url" value="${escapeAttr(m.base_url)}" /></td>
    <td><input class="m-model" value="${escapeAttr(m.model)}" /></td>
    <td><input class="m-env" value="${escapeAttr(m.api_key_env)}" /></td>
    <td><button class="ghost del">×</button></td>
  `;
  tr.querySelector(".del")!.addEventListener("click", () => tr.remove());
  return tr;
}

async function save() {
  const models: Record<string, ModelBlock> = {};
  root.querySelectorAll<HTMLTableRowElement>("#models-body tr").forEach((tr) => {
    const k = tr.querySelector<HTMLInputElement>(".m-key")!.value.trim();
    if (!k) return;
    models[k] = {
      provider: tr.querySelector<HTMLInputElement>(".m-provider")!.value.trim(),
      base_url: tr.querySelector<HTMLInputElement>(".m-url")!.value.trim(),
      model: tr.querySelector<HTMLInputElement>(".m-model")!.value.trim(),
      api_key_env: tr.querySelector<HTMLInputElement>(".m-env")!.value.trim() || "OPENAI_API_KEY",
      extra_body: null,
    };
  });

  const next: DesktopConfig = {
    agent: {
      controller_model: inputVal("ctrl-model") || "controller",
      executor_model: inputVal("exec-model") || "executor",
      reviewer_model: inputVal("rev-model") || null,
    },
    models,
    frontend: {
      capture_command: inputVal("capture-cmd") || null,
      preview_command: inputVal("preview-cmd") || null,
      max_visual_iterations: parseInt(inputVal("max-visual") || "5", 10),
      force: root.querySelector<HTMLInputElement>("#force-visual")!.checked,
    },
  };

  const status = root.querySelector<HTMLSpanElement>("#save-status")!;
  try {
    const path = await api.writeConfig(next);
    status.textContent = `Saved to ${path}`;
  } catch (e) {
    status.textContent = `Save failed: ${String(e)}`;
  }
}

function inputVal(id: string): string {
  return root.querySelector<HTMLInputElement>(`#${id}`)!.value.trim();
}

function escapeAttr(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
}
function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}
