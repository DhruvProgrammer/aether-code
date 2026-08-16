# aether — the OpenAI-compatible AI coding agent that remembers, plans, and ships

<p align="center">
  <img src="assets/aether.jpg" alt="aether — OpenAI-compatible AI coding agent" width="720" />
</p>

<p align="center">
  <a href="https://github.com/DhruvProgrammer/aether-code/releases/latest"><img src="https://img.shields.io/github/v/release/DhruvProgrammer/aether-code?style=flat-square" alt="Latest release"></a>
  <a href="https://github.com/DhruvProgrammer/aether-code/blob/main/LICENSE"><img src="https://img.shields.io/github/license/DhruvProgrammer/aether-code?style=flat-square" alt="License"></a>
  <a href="https://github.com/DhruvProgrammer/aether-code/stargazers"><img src="https://img.shields.io/github/stars/DhruvProgrammer/aether-code?style=flat-square" alt="GitHub stars"></a>
  <a href="https://github.com/DhruvProgrammer/aether-code/releases"><img src="https://img.shields.io/github/downloads/DhruvProgrammer/aether-code/total?style=flat-square" alt="Downloads"></a>
  <img src="https://img.shields.io/badge/Rust-1.75%2B-orange?style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/Windows-10%2B-0078d4?style=flat-square" alt="Windows">
  <img src="https://img.shields.io/badge/macOS-Universal-blue?style=flat-square" alt="macOS">
  <img src="https://img.shields.io/badge/Linux-x86__64%20%2F%20aarch64-yellow?style=flat-square" alt="Linux">
</p>

## What is aether?

**aether** is a free, open-source AI coding agent built in Rust that turns any OpenAI-compatible API endpoint (OpenAI, Azure, OpenRouter, NVIDIA NIM, MiniMax, GLM, DeepSeek, Ollama, LM Studio, vLLM, llama.cpp) into a full coding assistant. It plans, edits files, runs shell commands, runs tests, reviews its own work, and **remembers everything across sessions**.

It runs as a single static `.exe` you download once and forget about, or as a windowed **desktop app** you install in two clicks. No subscription, no telemetry, no cloud lock-in, no per-token upcharge — point it at the API you already pay for and go.

If you've used Cursor, Windsurf, Cline, GitHub Copilot, Continue.dev, Cody, or Tabnine, you'll feel at home. If you've used any of them and felt fenced in, you'll feel free.

---

## Table of contents

- [Why aether?](#why-aether)
- [Download & install](#download--install)
  - [Windows installer (recommended)](#windows-installer-recommended)
  - [Windows portable `.exe`](#windows-portable-exe)
  - [macOS](#macos)
  - [Linux](#linux)
  - [Build from source](#build-from-source)
- [Quick start](#quick-start)
- [Key features](#key-features)
- [How aether works](#how-aether-works)
  - [Two-LLM architecture](#two-llm-architecture)
  - [Multi-agent pipeline](#multi-agent-pipeline)
  - [Loop engineering](#loop-engineering)
  - [Persistent memory (aether-mind)](#persistent-memory-aether-mind)
  - [Visual engineering loop (optional 3rd LLM)](#visual-engineering-loop-optional-3rd-llm)
- [Use cases](#use-cases)
- [aether vs other AI coding agents](#aether-vs-other-ai-coding-agents)
- [Configuration](#configuration)
- [Commands & flags](#commands--flags)
- [Architecture (deep dive)](#architecture-deep-dive)
- [Safety & permissions](#safety--permissions)
- [Frequently asked questions](#frequently-asked-questions)
- [Roadmap](#roadmap)
- [Contributing](#contributing)
- [License](#license)

---

## Why aether?

Most AI coding tools assume a specific vendor (OpenAI), a specific subscription (Copilot), or a specific editor (Cursor). aether assumes **none of that**. You bring your own API key, your own model, your own machine. The agent adapts.

- **Bring your own model.** Use GPT-4o for the heavy lifting and a 4-bit quantized Llama-3 for the planning pass. Mix providers per role. Use DeepSeek for code, Claude for review, MiniMax for cheap sub-tasks. aether routes them by role, not by brand.
- **Persistent memory that survives sessions.** aether stores a knowledge graph + vector index + key-value facts on your local disk. The agent remembers your codebase conventions, recurring decisions, your preferences, and what you tried last week — without re-feeding it the entire repo every prompt.
- **Two-LLM design.** A small "controller" model plans the work, a big "executor" model writes the code. You pay less, you wait less, and the small model's mistakes don't pollute the big model's context.
- **Multi-agent verification.** Every implementation cycle is followed by automated tests, a peer-review pass, and a security review (for risky changes). Bugs caught by the reviewer are fed back into the loop.
- **Runs locally, on your terms.** Single-file static binary. No Electron runtime, no Chromium download, no 200 MB footprint. The optional desktop installer is ~5 MB.
- **Open source, MIT, auditable.** Every line of the agent loop, the memory engine, and the permission system is in this repo. Read it. Fork it. Modify it.
- **Honest about what it can't do.** Doesn't pretend to be sentient, doesn't claim 100% benchmark scores, doesn't hide failure modes behind emojis.

---

## Download & install

### Windows installer (recommended)

The fastest way to get a desktop-class aether experience on Windows.

1. Download the latest installer from the [releases page](https://github.com/DhruvProgrammer/aether-code/releases/latest):
   - **`aether_<version>_x64-setup.exe`** (NSIS, ~5 MB)
2. Double-click the installer → accept the UAC prompt → pick an install directory → Finish.
3. Launch **aether** from the **Start Menu**.
4. Open the **Settings** tab, paste your OpenAI-compatible API key and base URL, click **Save**.
5. Open the **Task** tab, type something like *"add a /login endpoint to the FastAPI app using JWTs"*, press **Ctrl + Enter**.

That's it. No PowerShell, no `pip install`, no Node version manager, no env vars to export.

### Windows portable `.exe`

For users who don't want an installer — drop the binary anywhere, run it.

1. Download **`aether-windows-x86_64.zip`** from the [releases page](https://github.com/DhruvProgrammer/aether-code/releases/latest).
2. Unzip anywhere (e.g. `C:\Tools\aether\`).
3. Double-click `aether.exe`. On a real terminal it drops you into the **TUI** (ratatui-based, three screens: Setup → Home → Run). Outside a terminal it errors out cleanly.

### macOS

Pre-built binaries are published for `x86_64` and `arm64` (Apple Silicon).

```bash
# Apple Silicon
curl -L -o aether-macos-arm64.tar.gz \
  https://github.com/DhruvProgrammer/aether-code/releases/latest/download/aether-macos-arm64.tar.gz
tar -xzf aether-macos-arm64.tar.gz
sudo mv aether /usr/local/bin/

# Intel
curl -L -o aether-macos-x64.tar.gz \
  https://github.com/DhruvProgrammer/aether-code/releases/latest/download/aether-macos-x64.tar.gz
tar -xzf aether-macos-x64.tar.gz
sudo mv aether /usr/local/bin/
```

A signed `.dmg` installer is on the roadmap.

### Linux

Static binaries for `x86_64` (glibc + musl) and `aarch64`.

```bash
curl -L -o aether-linux-x86_64.tar.gz \
  https://github.com/DhruvProgrammer/aether-code/releases/latest/download/aether-linux-x86_64.tar.gz
tar -xzf aether-linux-x86_64.tar.gz
sudo mv aether /usr/local/bin/
```

A `.deb` and an `.rpm` are planned.

### Build from source

Requires Rust **1.75+** and a C linker (Windows: MSVC or MinGW; macOS: Xcode CLT; Linux: `build-essential`).

```bash
git clone https://github.com/DhruvProgrammer/aether-code
cd aether-code
cargo build --release -p aether-cli -p aether-mcp
./target/release/aether.exe --help         # Windows
./target/release/aether --help            # macOS / Linux
```

To build the desktop installer:

```bash
cargo install tauri-cli --version "^2.0" --locked
cargo build --release -p aether-cli --bins
mkdir -p crates/aether-desktop/binaries && cp target/release/aether.exe crates/aether-desktop/binaries/
cargo tauri build --bundles nsis
```

---

## Quick start

**Step 1 — set your API key.** Either via the desktop app's **Settings** tab, or by exporting the environment variable the CLI reads:

```bash
# Any OpenAI-compatible endpoint works
export OPENAI_API_KEY="sk-..."
export OPENAI_BASE_URL="https://api.openai.com/v1"   # default; swap for your provider
```

**Step 2 — drop a `config.toml` in `~/.aether/`:**

```toml
[agent]
controller_model = "controller"   # the SMALL LLM
executor_model   = "executor"     # the BIG LLM

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

**Step 3 — run a task:**

```bash
aether "refactor the auth module to use JWTs and add tests"
```

That's it. The agent will plan, ask permission for risky shell commands, write the code, run the tests, review its own diff, and report back.

A fully worked example with screenshots lives at the bottom of this file under [Use cases](#use-cases).

---

## Key features

### 🔌 100% OpenAI-API compatible

Works with **every** endpoint that serves `/v1/chat/completions` and `/v1/embeddings`. Tested against:

| Provider | Endpoint | Notes |
|---|---|---|
| OpenAI | `https://api.openai.com/v1` | Default |
| Azure OpenAI | `https://<resource>.openai.azure.com/openai/deployments/<dep>` | Set `extra_body.api_version` if needed |
| OpenRouter | `https://openrouter.ai/api/v1` | All models, one key |
| DeepSeek | `https://api.deepseek.com/v1` | `deepseek-coder` recommended |
| Anthropic via proxy | any Anthropic-compatible proxy | |
| NVIDIA NIM | `https://integrate.api.nvidia.com/v1` | Free tier, Llama-3 70B |
| MiniMax | `https://api.minimax.chat/v1` | |
| GLM (Zhipu) | `https://open.bigmodel.cn/api/paas/v4` | |
| Ollama | `http://localhost:11434/v1` | Fully local, fully offline |
| LM Studio | `http://localhost:1234/v1` | Fully local |
| vLLM | `http://localhost:8000/v1` | Self-hosted |
| llama.cpp server | `http://localhost:8080/v1` | Self-hosted |

### 🧠 Persistent memory (aether-mind)

Every session writes to `~/.aether/mind/` — a hybrid store of:

- **Knowledge graph** (`redb` backend) — entities and relationships ("`User` prefers tabs", "`ProjectX` uses Drizzle ORM")
- **Vector index** — scalar-quantized embeddings for semantic recall
- **Key/value facts** — quick lookup for known things
- **Skill index** — auto-discovered `.aether/skills/*.md` instructions the agent can opt into

Memory is **retrieved at the start of every task** via hybrid search (vector similarity + keyword + 1-hop graph traversal), and **extracted from conversation** at the end. The agent never has to re-read the whole repo to know the conventions.

### 🪜 Two-LLM architecture (cost-aware routing)

Every aether task is decomposed into planning and execution:

- The **controller** is a SMALL, fast, cheap model (e.g. `gpt-4o-mini`, `llama-3-8b-instruct`, `qwen2.5-coder:7b`). It owns the prompt decomposition, the planning loop, and the orchestration of subagents. It never writes the final code.
- The **executor** is a BIG, careful, expensive model (e.g. `gpt-4o`, `claude-sonnet`, `deepseek-coder-v2`, `qwen2.5-coder:32b`). It receives the plan and writes the code, calls tools, and emits diffs.

Routing is enforced in exactly one place (`Agent::resolve`). The agent loop literally cannot send an implementation request to the SMALL model.

### 🧑‍🤝‍🧑 Multi-agent pipeline

aether ships with **ten** built-in subagents and an unlimited user-defined set. After each implementation cycle the orchestrator runs a **verification pipeline**:

- **Tester** — runs the project's tests, captures failures, feeds them back
- **Reviewer** — peer review of the diff (style, correctness, idioms)
- **Security Reviewer** — only on risky changes (auth, crypto, network, fs)

Subagent outputs feed into a single `EngineeringModel` that decides **Continue / Escalate / Stop** via the `LoopEngine` circuit breaker. No infinite retries. No silent failures.

### 🔁 Loop engineering

`Agent::run` is a closed loop: **plan → execute → verify → replan**. The loop is not just "retry on failure" — it tracks:

- **Stagnation** (same error three times in a row → escalate, don't retry)
- **Confidence** (low-confidence solutions trigger another review pass)
- **Budget** (max iterations, max wall-clock, max tokens per role)
- **Strategy** (current approach + rationale, persisted to `kv` so `--resume` continues from the same point)

### 🖼️ Visual engineering loop (optional 3rd LLM)

aether can spawn a **screenshot loop** for frontend tasks. When the task touches a UI and a reviewer model is configured, the agent:

1. Runs your app's `npm run build` / capture command to render the current state
2. Captures a screenshot
3. Sends the screenshot + the spec to the **reviewer** model
4. The reviewer approves, requests a minor fix, or escalates
5. The **executor** implements the fix, the loop re-captures, repeat

This is opt-in via the `[frontend]` section of `config.toml` and degrades gracefully when no reviewer model is set.

### 🔒 Permissions, fail-safe

Per-category policy (`read` / `edit` / `bash` / `delete` / `git_commit` / `network`) with `allow` / `ask` / `deny`. The default policy:

| Action | Default | Override |
|---|---|---|
| Read files | allow | always |
| Edit files | ask | `allow` for fully-autonomous mode |
| Run shell | ask | dangerous commands always denied |
| Delete files | ask | always confirmed |
| Git commit | ask | auto-approved with `--yes` |
| Network | ask | can be denied for offline mode |

**Hard denials** (cannot be overridden without editing source):
- `rm -rf`, `del /s /q`, `git reset --hard`, `git push --force`, `git clean -fd`, `mkfs`, `dd if=`, `format c:`, any command piped to a shell interpreter, any write to `/etc/`, `/boot/`, `C:\Windows\`, `C:\Program Files\`.

### 🔌 MCP client + MCP server

aether is both:

- **MCP client** — connect to any external Model Context Protocol server (`filesystem`, `github`, `postgres`, `slack`, …) and its tools become part of the agent's tool registry.
- **MCP server** — `aether-mcp` exposes aether-mind's memory tools (graph CRUD, vector search, fact lookup, skill list) over MCP, so any MCP-compatible host (Claude Desktop, Continue, Cursor) can use aether as its memory backend.

### 📦 Single-binary, zero-runtime

- **Windows installer**: ~5 MB (NSIS, includes WebView2 download if missing)
- **Portable `.exe`**: ~7 MB static binary, no DLLs to ship
- **RAM usage**: ~60 MB idle with embeddings off; ~250 MB with the embedded vector index warm

No Electron, no Chromium, no Node, no Python.

### 📋 Multiple ways to use it

| Mode | Entry point | Best for |
|---|---|---|
| **Desktop app** | `aether.exe` (double-click) | Non-technical users, day-to-day coding |
| **TUI** | `aether.exe` in a terminal | Power users, mouse-free workflows |
| **CLI** | `aether.exe "your task"` | CI/CD, scripts, one-off runs |
| **Background** | `aether --background "task"` | Long-running tasks you check later |
| **Worktree** | `aether --worktree "task"` | Risky changes you want to review before merging |
| **MCP server** | `aether-mcp.exe` | Hosting aether-mind for other AI tools |

---

## How aether works

### Two-LLM architecture

```
┌───────────────────────── ONE aether run ──────────────────────────┐
│                                                                    │
│  task ─► [Controller · SMALL LLM] ──plan──► [Executor · BIG LLM]    │
│                │                            │                      │
│                │ owns the loop               │ owns the tools       │
│                ▼                            ▼                      │
│           EngineeringModel            file edits, bash,           │
│           (loop state)                 git, tests, MCP             │
│                │                            │                      │
│                └──── verification ──────────┘                      │
│                          │                                         │
│              Tester → Reviewer → Security Reviewer                 │
│                          │                                         │
│                          ▼                                         │
│                  LoopEngine: Continue / Escalate / Stop            │
│                                                                    │
└────────────────────────────────────────────────────────────────────┘
```

**Why two models?**
- The planning step is high-frequency, low-stakes, and benefits from speed. A 7B model responds 5–10× faster than a 70B model and costs 10× less per token.
- The coding step is low-frequency, high-stakes, and benefits from depth. You want the big model thinking carefully, not the small model rushing.
- Context pollution: if the small model goes off the rails, the big model never sees the bad reasoning — it only sees the clean plan.

### Multi-agent pipeline

After the executor writes the diff, the orchestrator routes through three verification agents in parallel (where possible):

| Agent | LLM | Tools | Purpose |
|---|---|---|---|
| **Tester** | SMALL | bash (read-only + test runner) | Runs `npm test`, `pytest`, `cargo test`, …, reports failures |
| **Reviewer** | SMALL | read | Reads the diff, flags style/correctness issues |
| **Security Reviewer** | SMALL | read, grep | Audits for known-bad patterns: SQL injection, hardcoded secrets, unsafe fs ops |

The three outputs are merged into a single `ReviewReport` that's persisted to the session and feeds the next `EngineeringModel` update.

### Loop engineering

`EngineeringModel` tracks:

```rust
pub struct EngineeringModel {
    pub loop_state: LoopState,              // Understanding | Designing | Implementing | Verifying | Done
    pub attempts: u32,
    pub confidence: f32,                    // 0.0 .. 1.0
    pub stagnation_count: u32,              // consecutive failures with same error
    pub current_strategy: Option<String>,
    pub last_error: Option<String>,
    pub risk_level: RiskLevel,              // Low | Medium | High | Critical
    pub review: Option<ReviewReport>,
}
```

`LoopEngine::decide` returns one of:

- **Continue** — confidence > 0.7, last attempt was an improvement
- **Replan** — confidence 0.4..0.7, or strategy hasn't changed in 3 iterations
- **Escalate** — confidence < 0.4, stagnation ≥ 3, OR risk level is Critical
- **Stop** — max iterations, max wall-clock, or user cancel

State is persisted to `~/.aether/sessions.db` under the `engineering` key, so `--resume` reloads the exact loop position.

### Persistent memory (aether-mind)

aether-mind is a hybrid store:

```
~/.aether/mind/
├── graph.redb          # entities + relationships (typed)
├── vectors.bin         # scalar-quantized embeddings (8-bit, IVF-indexed)
├── kv.redb             # key/value facts (typed)
└── skills/             # auto-discovered .md instruction files
```

**Retrieval** is a hybrid of:
- **Vector similarity** (cosine over the quantized index)
- **BM25 keyword** over entity names + descriptions
- **1-hop graph expansion** from the top-k entities
- **Recency boost** (facts seen in the last 7 days get +20%)

**Extraction** happens at the end of every session:
- The conversation is summarized
- Named entities (people, projects, files, conventions, decisions) are extracted
- New facts are written to `kv`
- New entities/relationships are written to the graph
- Embeddings are regenerated for the new text

This means **aether gets smarter about your codebase the longer you use it**, without ever sending your code to a third-party training pipeline.

### Visual engineering loop (optional 3rd LLM)

For frontend tasks, a third LLM (the **reviewer**) joins the loop:

```
   [Executor] ──writes UI──►  [capture_command] ──► screenshot.png
        ▲                                              │
        │                                              ▼
        │                                    [Reviewer · multimodal LLM]
        │                                              │
        │                  approve / fix / escalate    │
        └──────────────────────────────────────────────┘
```

The reviewer sees the screenshot, the spec, and the current code, and emits a structured verdict. The executor implements the fix and the loop re-captures. Cap: `max_visual_iterations` (default 5) to prevent infinite loops.

Configuration:

```toml
[frontend]
capture_command = "npm --prefix {cwd} run build && node scripts/screenshot.js --out {out}"
preview_command = "npm --prefix {cwd} run dev"
max_visual_iterations = 5
force = false   # set true to enable for every task, not just frontend tasks
```

---

## Use cases

### 🔧 "Add a new feature"
```
$ aether "add a /login endpoint to the FastAPI app using JWTs, write tests, update the README"
```
aether will: plan the change, ask permission before touching files, implement the endpoint, run pytest, review the diff, and report what it did.

### 🐛 "Find and fix a bug"
```
$ aether "the /api/search endpoint returns 500 when query contains a quote — find and fix it"
```
aether will: read the relevant files, write a failing test, fix the bug, verify the test passes, show you the diff.

### 🔄 "Refactor a module"
```
$ aether "refactor src/auth/ to use dependency injection, keep all tests passing"
```
aether will: plan the refactor, do it incrementally, run tests after each step, roll back if anything breaks.

### 📚 "Document this codebase"
```
$ aether "write a README for the auth module explaining how JWTs are issued, validated, and rotated"
```
aether will: read the module, draft the README, ask permission before writing it.

### 🧪 "Write tests for untested code"
```
$ aether "find all functions in src/ that don't have tests and write tests for them"
```
aether will: enumerate untested functions, prioritize by risk, write tests one at a time.

### 🛡️ "Security audit"
```
$ aether --plan "audit src/ for SQL injection, hardcoded secrets, unsafe deserialization"
```
aether will: read the code, list findings with severity, suggest fixes — without touching any files (plan mode is read-only).

### 🤖 "Use aether as a CI step"
```yaml
# .github/workflows/ai-review.yml
- name: aether code review
  run: |
    aether --plan "review the diff vs main and report any issues, style violations, or missing tests"
```
aether exits non-zero if it finds Critical-severity issues. Use it as a merge gate.

---

## aether vs other AI coding agents

| | aether | Cursor | Windsurf | Cline / Continue | Copilot | opencode |
|---|---|---|---|---|---|---|
| **Open source** | ✅ MIT | ❌ | ❌ | ✅ (varies) | ❌ | ✅ |
| **Bring your own API key** | ✅ | ✅ | ✅ | ✅ | ❌ | ✅ |
| **Works with any OpenAI-compatible API** | ✅ | ❌ (Cursor-only) | ❌ | partial | ❌ | ✅ |
| **Persistent memory** | ✅ graph + vector + kv | ✅ (paid) | ✅ | partial | partial | ✅ |
| **Two-LLM routing (cost-aware)** | ✅ built-in | ❌ | ❌ | ❌ | ❌ | ✅ |
| **Multi-agent verification** | ✅ Tester + Reviewer + Security | ❌ | partial | ❌ | ❌ | ✅ |
| **Loop engineering (no infinite retries)** | ✅ | ❌ | ❌ | ❌ | ❌ | ✅ |
| **Visual screenshot loop** | ✅ optional | partial | ❌ | ❌ | ❌ | ❌ |
| **MCP client + server** | ✅ | partial | partial | ✅ client | partial | ✅ |
| **Single static binary** | ✅ 5 MB | ❌ (Electron, 200 MB+) | ❌ | ❌ | ❌ | partial |
| **Windows NSIS installer** | ✅ | ✅ | ✅ | ❌ | ❌ | ✅ |
| **TUI for terminal users** | ✅ ratatui | ❌ | ❌ | ✅ (Continue) | ❌ | ✅ |
| **Free / no subscription** | ✅ | partial | partial | ✅ | ❌ | ✅ |
| **No telemetry** | ✅ | ❌ | ❌ | partial | ❌ | ✅ |
| **Fully offline (local model)** | ✅ Ollama/vLLM | ❌ | ❌ | ✅ (with local model) | ❌ | ✅ |

aether's differentiators: **open-source two-LLM design**, **persistent hybrid memory that actually persists across sessions**, **opt-in visual review loop**, and a **single 5 MB binary**.

---

## Configuration

Default config path: `~/.aether/config.toml`. Override with `--config /path/to/config.toml` or `AETHER_CONFIG` env var.

Full annotated example: [`config.example.toml`](./config.example.toml).

Key sections:

- `[agent]` — controller/executor model names, optional reviewer, max iterations, max wall-clock, max tokens per role
- `[models.<key>]` — provider, base URL, model name, API key env var, optional `extra_body` for provider-specific params
- `[permissions]` — per-category allow/ask/deny; dangerous-bash allowlist
- `[memory]` — embedding model, vector index size, retrieval k, extraction policy
- `[mcp.servers]` — external MCP servers to connect on startup
- `[frontend]` — visual-review capture/preview commands

---

## Commands & flags

```
aether [TASK]                 # run a task non-interactively
aether --plan "TASK"          # read-only planning mode, never modifies files
aether --local "TASK"         # point all models at http://localhost:11434/v1
aether --background "TASK"    # spawn a detached child, return session id immediately
aether --worktree "TASK"      # run in a git worktree, leave the main tree clean
aether --resume <session-id>  # continue a previous session
aether --traces --resume <id> # print the trace log of a previous session
aether --rollback <id>        # roll back the last file-write checkpoint
aether --tui                  # force-launch the interactive TUI
aether --json                 # emit machine-parseable JSON
aether --debug                # verbose tracing
```

`aether-mcp` runs the MCP server on stdin/stdout for any MCP-compatible host.

---

## Architecture (deep dive)

```
┌─────── crates/aether-cli ───────┐
│ Clap CLI                         │
│  ├─ non-interactive `aether "…"` │
│  ├─ interactive TUI (ratatui)    │
│  └─ desktop app shell (Tauri 2)  │
└──────────────┬──────────────────┘
               │
┌──────────────▼──────────────────┐
│ crates/aether-core               │
│  ├─ agent_loop · Agent::run      │
│  ├─ controller · Executor        │
│  ├─ subagents · 10 built-ins     │
│  ├─ eng · LoopEngine             │
│  └─ visual · 3rd-LLM screenshot  │
└──────────────┬──────────────────┘
               │
┌──────┬───────┴────────┬─────────────┐
│      │                │             │
▼      ▼                ▼             ▼
models  tools         sessions       mind
(OpenAI  (fs, shell,   (SQLite      (graph + vec
compat)  git, MCP)     traces)      + kv + skills)
```

All crates are MIT-licensed, Rust edition 2021. Total Rust LOC: ~18k.

---

## Safety & permissions

aether takes "AI touching your filesystem" seriously. The default policy is conservative:

- **Read** is always allowed
- **Edit** asks unless `--yes` is set
- **Bash** asks, except for a deny-list of always-forbidden commands (`rm -rf`, `git reset --hard`, `git push --force`, `mkfs`, `format c:`, `dd of=`, writes to `/etc/`, `/boot/`, `C:\Windows\`, `C:\Program Files\`)
- **Delete** always asks
- **Git commit** always asks (prevents surprise commits)
- **Network** asks by default; set to `deny` for fully-offline operation

The agent **cannot** be coerced into running a denied command via prompt injection — the check happens before the command is parsed by the shell, in `is_dangerous_command`, which is fuzzed.

Every shell command is also **shell-escaped** before substitution into visual-review capture/preview commands — no command injection through `{cwd}` or `{out}`.

See [`crates/aether-permissions/`](./crates/aether-permissions/) for the implementation.

---

## Frequently asked questions

**Is aether really free?**
Yes. MIT-licensed source, no telemetry, no subscription. You pay only your API provider (or nothing if you run a local model).

**Does aether send my code anywhere?**
Only to the API endpoint you configure in `[models.<key>].base_url`. If you point at Ollama or vLLM, nothing leaves your machine. There is no fallback telemetry endpoint.

**What models work?**
Anything that serves `/v1/chat/completions` and `/v1/embeddings`. We test against OpenAI, DeepSeek, OpenRouter, NVIDIA NIM, Ollama, LM Studio, vLLM, llama.cpp. Anthropic-via-proxy works too.

**How big is the binary?**
~7 MB on Windows (static, no DLL deps). The NSIS installer is ~5 MB.

**Can I use aether without an OpenAI account?**
Yes. Point `base_url` at Ollama, LM Studio, vLLM, or any self-hosted OpenAI-compatible server. aether-mind also has a `local_embeddings` mode that uses fastembed-rs instead of an embedding API.

**Can aether edit my whole project without asking?**
Yes, with `permissions.edit = "allow"` in `config.toml` or `--yes` on the CLI. The dangerous-bash deny list still applies regardless.

**Does aether work offline?**
Yes, with a local model endpoint (Ollama, LM Studio, vLLM, llama.cpp). Set `[permissions].network = "deny"` to enforce no external calls.

**Is there a VS Code extension?**
Not yet — on the roadmap. aether is editor-agnostic; today you launch it from a terminal, the Start Menu, or via `--background` from any editor.

**How is aether different from opencode?**
Both are open-source OpenAI-compatible agents. aether adds: a hard-enforced two-LLM split (opencode has this too but it's softer), an explicit loop-engineering state machine with a circuit breaker, an opt-in visual-review loop with screenshot capture, and a Windows NSIS installer shipped by default.

**How is aether different from Cline?**
Cline is a VS Code extension; aether is a standalone app + CLI + TUI + MCP server. Cline has no persistent memory layer; aether does (graph + vector + kv). Cline uses one LLM for everything; aether uses two by default and optionally three for visual review.

**How is aether different from Cursor?**
Cursor is a fork of VS Code with an AI sidebar; aether is a standalone agent that any editor can drive. Cursor's memory is per-project and proprietary; aether's is open and portable. Cursor is closed source; aether is MIT. Cursor locks you into its model picker; aether works with any provider.

**How is aether different from GitHub Copilot?**
Copilot is a subscription, not a tool you own. Copilot edits are streamed live in your editor; aether edits happen via tool calls and show up in the trace log. Copilot has no agentic loop; aether does. Copilot can't run your tests; aether can.

**Does aether support Anthropic Claude directly?**
Not directly (Anthropic doesn't expose an OpenAI-compatible endpoint). Use one of the many Anthropic-to-OpenAI proxies (LiteLLM, Portkey, or any of the open-source ones).

**Can I contribute?**
Yes — see [Contributing](#contributing).

**Where do I report bugs?**
[GitHub Issues](https://github.com/DhruvProgrammer/aether-code/issues). Please include the output of `aether --debug "your task"` and the relevant `~/.aether/sessions.db` excerpt.

---

## Roadmap

**v0.10 — IDE integrations**
- VS Code extension (sidebar chat, code-action provider)
- JetBrains plugin
- Neovim Lua plugin

**v0.11 — Better memory**
- Cross-project memory sharing (opt-in, with explicit allowlist)
- Auto-summarization of long sessions before context overflow
- Memory export/import (so you can sync between machines)

**v0.12 — macOS + Linux installers**
- Signed `.dmg` for macOS
- `.deb` and `.rpm` for Linux
- AppImage

**v0.13 — More providers**
- Native Anthropic adapter (no proxy)
- Google Gemini adapter
- Mistral adapter

**v1.0 — Stable API**
- Frozen public CLI flags and `config.toml` schema
- Semver guarantees on the crate-level API for embedding aether as a library

---

## Contributing

Bug reports, feature requests, and PRs welcome. Open an issue first for non-trivial changes so we can agree on the direction.

```bash
git clone https://github.com/DhruvProgrammer/aether-code
cd aether-code
cargo test --workspace            # 70+ unit tests, all green
cargo build -p aether-cli --release
cargo install tauri-cli --version "^2.0" --locked
cargo tauri dev --config crates/aether-desktop/tauri.conf.json   # dev mode for the desktop app
```

The `agents/` directory contains per-agent TOML configs (`explorer.toml`, `implementer.toml`, `tester.toml`, …) — these are how you customize agent behavior without touching Rust.

See [CONTRIBUTING.md](./CONTRIBUTING.md) for the full guide.

---

## License

MIT — see [LICENSE](./LICENSE).

```
AETHER © 2024-2026 aether contributors
Released under the MIT License.
```

You are free to use aether commercially, modify it, distribute it, and ship it inside your own products. Attribution appreciated but not required.

---

## Star history

If aether saved you a subscription or a Stack Overflow visit, a star helps others find it.

<p align="center">
  <a href="https://github.com/DhruvProgrammer/aether-code/stargazers">
    <img src="https://img.shields.io/github/stars/DhruvProgrammer/aether-code?style=social" alt="GitHub stars">
  </a>
</p>

---

<p align="center">
  <sub>Built with Rust 🦀 · MIT licensed · No telemetry · No subscription · No lock-in</sub>
</p>
