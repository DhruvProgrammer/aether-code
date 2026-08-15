import { api } from "./api";
import { mountTask, unmountTask } from "./views/task";
import { mountSettings, unmountSettings } from "./views/settings";
import { mountHistory, unmountHistory } from "./views/history";
import "./styles.css";

const VIEWS = ["task", "settings", "history"] as const;
type View = typeof VIEWS[number];

const viewMounts: Record<View, () => Promise<void>> = {
  task: mountTask,
  settings: mountSettings,
  history: mountHistory,
};
const viewUnmounts: Record<View, () => void> = {
  task: unmountTask,
  settings: unmountSettings,
  history: unmountHistory,
};

let current: View = "task";

document.querySelectorAll<HTMLButtonElement>("aside nav button").forEach((btn) => {
  btn.addEventListener("click", async () => {
    const next = btn.dataset.view as View;
    if (next === current) return;
    viewUnmounts[current]();
    document.querySelector(`#view-${current}`)?.classList.add("hidden");
    document.querySelector(`aside nav button[data-view="${current}"]`)?.classList.remove("active");
    current = next;
    btn.classList.add("active");
    document.querySelector(`#view-${current}`)?.classList.remove("hidden");
    await viewMounts[current]();
  });
});

// Initial mount.
viewMounts[current]();

// Footer version label.
api.version().then((v) => {
  const f = document.querySelector<HTMLDivElement>("#footer");
  if (f) f.textContent = `v${v}`;
});
