# AETHER

> Lightweight, Rust-native, OpenAI-API-compatible coding agent with an embedded persistent memory engine.

AETHER is a coding agent that talks to **any** OpenAI-compatible API (`/v1/chat/completions` + `/v1/embeddings`) and ships as a single small Windows `.exe`. It uses a two-model architecture — a **Controller** that plans and an **Executor** that codes — plus an embedded memory engine (`aether-mind`) that remembers users, projects, and facts across sessions so context never gets lost in a flat transcript.

## Highlights

- **100% OpenAI-compatible.** Point it at OpenAI, Azure OpenAI, OpenRouter, NVIDIA NIM, MiniMax, GLM, vLLM, Ollama, or LM Studio — anything that serves `/v1/chat/completions`. No vendor lock-in.
- **Two-LLM design (spec 1).** The Controller decomposes the task and writes a plan; the Executor implements it by calling tools. Read-only or explanatory tasks can be routed to a cheaper model (cost routing, 8).
- **Memory-first (spec 9).** `aether-mind` stores a knowledge graph (`redb`), key/value facts, and a scalar-quantized vector index for semantic recall. Retrieval fuses vector similarity + keyword + 1-hop graph traversal.
- **Reviewer / Tester subagents (spec 7).** After the Executor finishes, an optional Reviewer and Tester run structured handoff passes and feed diffs/results back to the Executor.
- **Permissions + planning mode (spec 14).** Per-category policy (`read`/`edit`/`bash`/`delete`/`git_commit`/`network`) with `allow`/`ask`/`deny`. `--plan` runs read-only and returns a plan without touching files.
- **MCP client and server (spec 6).** `aether` can connect to external MCP servers and use their tools; `aether-mcp` is itself an MCP server exposing the memory tools.
- **Local / cloud mode.** `--local` redirects every model at a local OpenAI-compatible endpoint (Ollama / llama.cpp by default) with zero config changes.
- **Minimalist light UI.** Pantone-anchored, low-chroma terminal styling (`docs/design.md`). No emojis, no noise.
- **Small and dependency-justified.** A single static `.exe`; every dependency is justified against the spec (30). RAM target is ~60 MB with embeddings off.

## Why "aether"?

AETHER is to coding agents what a clean, dependency-light native binary is to the usual Node/Python stacks: one self-contained `.exe`, no runtime to install, no telemetry, and a memory layer that remembers between sessions instead of re-reading the whole conversation every time.

## Architecture

```
            +-------------+
 prompt --> |  Controller |  plans, writes a numbered plan, selects tools
            +-----+-------+
                  | plan
            +-----v-------+
         +->  Executor  -- calls tools (fs / terminal / git / memory / mcp) -+
         |    (coder)                                                       |
         |                                                                  v
         |  optional handoff:  Reviewer --> Tester --> (diffs/results back to Executor)
         +------------------------------------------------------------------+
                              | uses
                       +------v-------+
                       |  aether-mind |  redb graph + kv + quantized vector, hybrid retrieval
                       +--------------+
```

Workspace crates (all MIT, edition 2021):

| Crate | Responsibility |
|-------|----------------|
| `aether-config` | TOML config loading: agent, models, memory, permissions, context, display, subagents, MCP. |
| `aether-permissions` | `Permission` enum plus `Policy` (category to allow/ask/deny) and dangerous-bash detection. |
| `aether-models` | OpenAI-compatible provider: streaming `/v1/chat/completions` + `/v1/embeddings`. |
| `aether-tools` | `Tool` trait plus filesystem/terminal/git tools, plus the MCP client adapter. |
| `aether-sessions` | SQLite session + checkpoint store (`~/.aether/sessions.db`). |
| `aether-mind` | Memory engine: graph, key/value, quantized vector index, retrieval, skills, extraction. Also the `aether-mcp` server binary. |
| `aether-core` | The agent loop: Controller/Executor, subagents, context compaction, cost routing, checkpoints. |
| `aether-cli` | CLI entrypoint plus Pantone UI. Compiles the `aether` and `aether-mcp` binaries. |

## Requirements and building

- **Rust** 1.80+ (tested on 1.97) with the `x86_64-pc-windows-gnu` target.
- **MinGW-w64** (provides `gcc` / `ld` for linking). On Windows, install MSYS2 MinGW and put `C:\mingw64\bin` on `PATH`.
- No external services are required at build time.

Build:

```powershell
$env:Path = "C:\mingw64\bin;" + $env:USERPROFILE + "\.cargo\bin;" + $env:Path
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "gcc"
cargo build --release
```

This produces:

- `target/release/aether.exe` — the agent.
- `target/release/aether-mcp.exe` — the `aether-mind` MCP server.

### Standalone .exe

The release profile uses `-C target-feature=+crt-static`, so the binary statically links the C runtime and does **not** need `libgcc_s_seh-1.dll` / `libstdc++-6.dll` / `libwinpthread-1.dll` next to it. (If you link without `crt-static`, keep those MinGW DLLs alongside the `.exe`.) The binary still depends only on Windows system DLLs that are always present; a fully MSVC-static build would require the Visual Studio build tools, which this project does not assume.

## Installation and configuration

1. Copy `config.example.toml` to `%USERPROFILE%\.aether\config.toml`.
2. Add at least a `controller` and `executor` model pointing at your OpenAI-compatible endpoint, and set `api_key_env` to the **name of an environment variable** that holds your key. **No keys are ever written to disk** — AETHER only stores the env-var name.
3. Run `aether "<your task>"`.

Minimal config:

```toml
[agent]
controller_model = "controller"
executor_model   = "executor"

[models.controller]
provider    = "openai_compatible"
base_url    = "https://api.openai.com/v1"
model       = "gpt-4o-mini"
api_key_env = "OPENAI_API_KEY"

[models.executor]
provider    = "openai_compatible"
base_url    = "https://api.openai.com/v1"
model       = "gpt-4o"
api_key_env = "OPENAI_API_KEY"
```

Then in your shell: `set OPENAI_API_KEY=sk-...` (or `$env:OPENAI_API_KEY = "..."` in PowerShell).

### Configuration reference

See `config.example.toml` for every field. Key groups:

- **`[agent]`** — `controller_model`, `executor_model`, `max_iterations`, `routing_policy`, optional `cheap_model` (cost routing), `local_endpoint` (used by `--local`, default `http://127.0.0.1:11434/v1`).
- **`[models.*]`** — one table per named model: `provider` (only `openai_compatible` in v1), `base_url`, `model`, `api_key_env`.
- **`[memory]`** — `enabled`, `memory_top_k`, `path` (`~/.aether/mind.redb`), `auto_extract` (opt-in LLM fact extraction, default `false`).
- **`[permissions]`** — `read`/`edit`/`bash`/`delete`/`git_commit`/`network` -> `allow`/`ask`/`deny`.
- **`[context]`** — `max_tokens` (context compaction budget).
- **`[display]`** — `theme = "light"`, `accent`, `emoji = false`.
- **`[subagents]`** — `enabled`, `reviewer_model`, `tester_model`.
- **`[[mcp.servers]]`** — external MCP servers: `name`, `command`, `args`.

## Usage

> **AETHER is a command-line tool, not a GUI app.** Double-clicking `aether.exe` opens a console window that prints setup help (and waits for Enter) — it does not open a graphical window. To use AETHER, open a terminal (CMD or PowerShell) and run it with a task, e.g. `aether "explain the main loop in src/main.rs"`. With no arguments it starts an interactive prompt.

```text
aether "<task>"            Run a task non-interactively
aether                     Start the interactive REPL (type /exit to quit)
aether --plan "<task>"     Read-only planning mode (returns a plan, no file changes)
aether --local "<task>"    Point all models at the local endpoint (Ollama / llama.cpp)
aether --rollback <id>     Restore the last file-write checkpoint for a session
aether --config path.toml  Use a specific config file
aether --json "<task>"     Emit machine-parseable JSON (plan / result / review / test)
```

### Memory tools

Once memory is enabled, these tools are available to the agent (and to MCP clients):

- `memory_save` — persist a node (`user` / `project` / `episodic` / `skill`) and optional relations.
- `memory_query` — hybrid retrieval (vector + keyword + graph) for a query.
- `memory_forget` — delete a node and its edges.
- `skill_search` — search discovered `SKILL.md` skills by name/description.

AETHER also auto-discovers `AGENTS.md` / `CLAUDE.md` / `AETHER.md` / `CONTEXT.md` in the working directory as durable project context.

### Subagents

Enable `[subagents] enabled = true` to run a Reviewer (reads the plan + final diff, suggests corrections) and a Tester (proposes/verifies runnable checks) after the Executor. Their structured results are handed back to the Executor for a fix pass. Reviewer/Tester are **read-only** roles and cannot modify files.

### Checkpoints and rollback

Every file write snapshots the previous content into `~/.aether/sessions.db`. If the agent changes something you dislike:

```text
aether --rollback <session_id>
```

restores the most recent pre-write state (or removes the file if it did not exist before).

### Local mode

```text
ollama serve            # or: ollama run llama3
aether --local "refactor src/main.rs"
```

`--local` rewrites every model's `base_url` to `agent.local_endpoint` (default `http://127.0.0.1:11434/v1`), which is exactly what Ollama and llama.cpp expose. No other changes needed.

### MCP

- **Use external MCP servers:** add `[[mcp.servers]]` entries; `aether` connects at startup and registers their tools. Connection failures are non-fatal.
- **Expose memory as MCP:** run `aether-mcp` (stdio JSON-RPC). Any MCP client can call `memory_save` / `memory_query` / `memory_forget` / `skill_search`.

```text
echo {"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}} | aether-mcp
```

## Design

The terminal UI follows a minimalist, low-chroma **Pantone** palette (Still Blue `#8EAFBB` accent, Pavement `#3A3A3C` ink, Cloud Grey `#8C9296` muted, Marigold `#C08A2D` warn, Red Maple `#9E4A45` error). Full tokens and rationale are in `docs/design.md`.

## Status

All six phases of `docs/plan.md` are implemented:

1. Core two-LLM agent (Controller/Executor), OpenAI-compatible provider, fs/terminal tools.
2. Permissions engine plus planning mode, git tools, SQLite sessions.
3. `aether-mind` memory engine (graph + quantized vector + hybrid retrieval + skills + context discovery), memory tools.
4. Subagent orchestration (Reviewer/Tester handoff).
5. Context compaction, cost routing, checkpoints/rollback, Pantone UI.
6. MCP client, `aether-mind` as MCP server, local/cloud modes, quantized index.

See `docs/plan.md` for the phased roadmap and `DEPENDENCIES.md` for the dependency ledger (including deferred `usearch` / `ratatui`).

## Docs

- `docs/plan.md` — implementation roadmap and per-phase status.
- `docs/design.md` — Pantone design system.
- `docs/config.md` — configuration reference.
- `DEPENDENCIES.md` — every dependency and why it is justified (or deferred).

## Security, SmartScreen and code signing

**AETHER is not malware.** When you first run the downloaded `.exe`, Windows may show a *"Windows protected your PC"* / SmartScreen dialog. That is **not** a virus detection — Windows Defender's malware engine does **not** flag AETHER (verify on your own machine with `MpCmdRun.exe -Scan -ScanType 3 -File aether.exe`). The warning appears because the prebuilt binary is **unsigned** (unknown publisher); SmartScreen blocks unknown-publisher downloads until the publisher earns reputation.

How to run it safely:

1. **Run it anyway.** In the SmartScreen dialog, click *More info* -> *Run anyway*. The file is safe.
2. **Build from source (no warning at all).** A binary you compile yourself is not "downloaded from the internet", so Windows does not attach the Mark-of-the-Web and SmartScreen will not warn. See [Requirements and building](#requirements-and-building).
3. **Use a signed release.** Code signing removes the warning entirely. The repo ships a GitHub Actions workflow (`.github/workflows/release.yml`) that builds with the standard MSVC toolchain and **automatically code-signs** the binaries if you provide a certificate:

   - Obtain an Authenticode code-signing certificate (an **EV** certificate gives instant SmartScreen reputation; a standard **OV/IV** certificate builds reputation as downloads accumulate).
   - In *Repository settings -> Secrets and variables -> Actions*, add:
     - `WINDOWS_CODESIGN_PFX` — the certificate exported as a **base64-encoded PFX** (`certutil -encode cert.pfx cert.b64`, then paste the file contents).
     - `WINDOWS_CODESIGN_PASSWORD` — the PFX password.
   - Publish a release (or run the *Release (Windows)* workflow manually). The `.exe` files are signed with a SHA-256 timestamp, and SmartScreen stops warning. Until signing is configured, the release binaries are uploaded **unsigned** and SmartScreen will still warn.

The MSVC build in CI also avoids the MinGW toolchain entirely, which further reduces false-positive heuristic flags from some antivirus products.

## License

MIT. See `LICENSE`.
