import { api, type MessageRow, type SessionRow } from "../api";

const root = document.querySelector<HTMLElement>("#view-history")!;

let sessions: SessionRow[] = [];
let active: string | null = null;

export async function mountHistory() {
  await refresh();
}

export function unmountHistory() {}

async function refresh() {
  sessions = await api.listSessions();
  await render();
}

async function render() {
  const items = sessions.length
    ? sessions
        .map((s) => {
          const id = escapeAttr(s.id);
          const title = escapeHtml((s.task ?? "(no task)").slice(0, 80));
          const meta = escapeHtml(s.created_at);
          const cls = active === s.id ? "active" : "";
          return `<li data-id="${id}" class="${cls}">
              <div class="title">${title}</div>
              <div class="meta">${meta}</div>
            </li>`;
        })
        .join("")
    : `<li class="empty">No past sessions yet.</li>`;

  root.innerHTML = `
    <div class="card">
      <h2>History</h2>
      <div class="split">
        <ul class="session-list" id="session-list">${items}</ul>
        <div class="session-detail" id="session-detail">
          <p class="muted">Select a session to view its messages.</p>
        </div>
      </div>
    </div>
  `;
  root.querySelectorAll<HTMLLIElement>("#session-list li[data-id]").forEach((li) => {
    li.addEventListener("click", async () => {
      active = li.dataset.id!;
      await loadDetail(active);
      await render();
    });
  });
  if (active) await loadDetail(active);
}

async function loadDetail(id: string) {
  const detail = root.querySelector<HTMLDivElement>("#session-detail")!;
  detail.innerHTML = `<p class="muted">Loading…</p>`;
  try {
    const messages = await api.getSessionMessages(id);
    const body = messages.length
      ? messages.map(renderMessage).join("")
      : `<p class="muted">No messages recorded.</p>`;
    detail.innerHTML = `
      <h3>${escapeHtml(id)}</h3>
      <div class="messages">${body}</div>
    `;
  } catch (e) {
    detail.innerHTML = `<p class="error">Failed: ${escapeHtml(String(e))}</p>`;
  }
}

function renderMessage(m: MessageRow): string {
  return `<div class="msg msg-${escapeAttr(m.role)}">
    <div class="msg-head"><span class="role">${escapeHtml(m.role)}</span> <span class="ts muted">${escapeHtml(m.ts)}</span></div>
    <pre>${escapeHtml(m.content)}</pre>
  </div>`;
}

function escapeAttr(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
}
function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}
