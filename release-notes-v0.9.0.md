## v0.9.0 — Desktop App (Tauri + WebView2 installer)

aether is now a real desktop application, not just a CLI. Run the installer below and you get a windowed app in your Start Menu — no terminal, no PowerShell, no setup script. Modeled after opencode's desktop bundle but built with **Tauri 2** (Rust + WebView2) instead of Electron, so the installer stays small (~5 MB) and the binary stays a single Rust codebase.

### What you get

* **`aether_0.9.0_x64-setup.exe`** — a Windows NSIS installer that:
  * Adds **aether** to your Start Menu and (optionally) Desktop
  * Installs `aether.exe` + `aether-mcp.exe` to `C:\Program Files\aether\`
  * Bundles WebView2 (Windows 10+ already has it; otherwise the installer fetches it)
* After install, launch **aether** from the Start Menu — a real windowed app opens with three views in a sidebar:
  * **Task** — paste a task, hit `Ctrl+Enter`, watch the agent run live with streaming output
  * **Settings** — full editor for `~/.aether/config.toml`: API key, model table, controller/executor/reviewer routing, visual-review capture/preview commands. Save button writes the file.
  * **History** — list of past sessions from `~/.aether/sessions.db`; click one to see its full message log

### Architecture

* New crate **`crates/aether-desktop`** — a Tauri 2 binary that wraps the existing `aether.exe` CLI:
  * `read_config` / `write_config` — direct TOML I/O on `~/.aether/config.toml`
  * `list_sessions` / `get_session_messages` — read the same SQLite DB the CLI writes to
  * `run_task` — spawns the bundled `aether.exe` and streams its stdout/stderr to the webview as `task-output` events
  * `cancel_task` — terminates the child via `taskkill` on Windows
* New web frontend **`packages/app`** — TypeScript + Vite (~11 KB JS):
  * 3-view SPA with sidebar nav, dark theme
  * Communicates with the Rust backend only through Tauri's typed `invoke` IPC
* New GitHub Actions workflow **`.github/workflows/desktop.yml`** — builds the Windows installer on every tag push and attaches it to the GitHub release.

### Existing CLI / TUI / MCP server

Untouched. `aether.exe` (CLI), `aether-mcp.exe` (MCP server), and the ratatui TUI from v0.8.0 all keep working exactly as before. The desktop app simply calls `aether.exe` as a child process, so it shares the same engine.

### Try it

1. Download `aether_0.9.0_x64-setup.exe` below.
2. Double-click, accept the UAC prompt, pick an install dir, Finish.
3. Launch **aether** from the Start Menu.
4. Go to **Settings**, enter your `OPENAI_API_KEY` and a base URL, hit Save.
5. Go to **Task**, type something like "refactor the auth module to use JWTs", press `Ctrl+Enter`.

Workspace version bumped to **0.9.0**.
