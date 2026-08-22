# AETHER

**An open-source, multi-LLM AI coding agent built around specialized planning, execution, and independent review — with persistent task intelligence and realtime workspace awareness.**

AETHER is an autonomous AI software engineering agent that separates the three hardest problems of AI-assisted coding into three dedicated LLM roles: one model understands and reviews, one model plans and orchestrates, and one model executes and builds. The result is a controlled Understand → Plan → Execute → Review → Verify engineering loop with persistent task state, context compaction, realtime Git-aware changes, and explicit provider isolation for any OpenAI-compatible model.

<p align="center">
  <a href="https://github.com/DhruvProgrammer/aether-code/releases/latest"><img src="https://img.shields.io/github/v/release/DhruvProgrammer/aether-code?style=flat-square" alt="Latest release"></a>
  <a href="https://github.com/DhruvProgrammer/aether-code/blob/main/LICENSE"><img src="https://img.shields.io/github/license/DhruvProgrammer/aether-code?style=flat-square" alt="License: MIT"></a>
  <a href="https://github.com/DhruvProgrammer/aether-code/stargazers"><img src="https://img.shields.io/github/stars/DhruvProgrammer/aether-code?style=flat-square" alt="GitHub stars"></a>
  <img src="https://img.shields.io/badge/Rust-1.75%2B-orange?style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/Windows-10%2B-0078d4?style=flat-square" alt="Windows">
  <img src="https://img.shields.io/badge/macOS-Universal-blue?style=flat-square" alt="macOS">
  <img src="https://img.shields.io/badge/Linux-x86__64%20%2F%20aarch64-yellow?style=flat-square" alt="Linux">
</p>

<p align="center">
  <img src="assets/aether.jpg" alt="AETHER — multi-LLM AI coding agent" width="720" />
</p>

> If you're interested in open-source AI coding agents, multi-LLM orchestration, and autonomous software engineering, star AETHER and follow the project.

---

## Why AETHER Is Different

Most AI coding tools ask one model to do everything: understand the task, create the plan, write the code, and judge its own work. AETHER separates these responsibilities across three specialized LLM roles.

```
                    AETHER
                      │
              ┌───────┴───────┐
              │               │
           LLM 3           LLM 2
        Observer/        Planner/
         Reviewer        Orchestrator
              │               │
              └───────┬───────┘
                      ↓
                    LLM 1
               Executor/Builder
```

**Planner ≠ Executor ≠ Reviewer.** This is the core AETHER concept.

- The model that writes the code does not grade its own work.
- The model that plans does not directly modify files.
- The model that reviews is independent from the model that implemented.
- Completion requires actual verification evidence — never self-declared success.

Each role is bound to its own provider and model via explicit configuration. There is no routing, no fallback, no automatic model switching. You decide which model performs which role.

---

## How AETHER Works

```
User Request
     ↓
LLM 3 — Understand
     ↓
LLM 2 — Plan
     ↓
LLM 1 — Execute
     ↓
LLM 3 — Review & Verify
     ↓
   ┌───────┐
   │ PASS? │
   └───┬───┘
       │
   ┌───┴────┐
   │        │
  YES      NO
   │        │
   ↓        ↓
 DONE     LLM 2
          Replan
             ↓
          LLM 1
          Repair
             ↓
          LLM 3
          Verify
```

When you send a task to AETHER:

1. **LLM 3 (Observer)** inspects the workspace, understands the objective, identifies relevant files and constraints.
2. **LLM 2 (Planner)** receives the structured understanding and produces an execution plan with steps, dependencies, and verification requirements.
3. **LLM 1 (Executor)** implements the plan — creating, modifying, and running tools under AETHER's permission engine.
4. **LLM 3 (Reviewer)** independently reviews the result against the original objective, inspects diffs and test output, and determines whether verification passes.
5. If verification fails, **LLM 2 replans** with the failure evidence, **LLM 1 repairs**, and **LLM 3 re-verifies**.

This loop is bounded. Repair attempts and replans have configurable limits. Doom-loop detection identifies repeated identical failures and forces strategy changes or stops the task with preserved evidence.

---

## The Three LLMs

### LLM 1 — Executor / Builder

The hands-on implementation model. LLM 1 creates files, modifies code, runs shell commands, executes tests, and applies repairs. It is the only role that changes the workspace.

LLM 1 **cannot** declare the overall task complete. It reports step completion; LLM 3 decides whether the task is done.

### LLM 2 — Planner / Orchestrator

The planning and decomposition model. LLM 2 analyzes objectives, creates execution plans, identifies dependencies, determines execution order, defines verification requirements, and replans when verification fails.

LLM 2 does not directly modify project files.

### LLM 3 — Observer / Reviewer / Verifier

The independent review model. LLM 3 understands the initial request, reviews plans, inspects implementation results, analyzes tool output and Git diffs, and concludes whether the task is complete.

Only LLM 3 may conclude task completion, and only with actual verification evidence (test results, build output, typecheck status). LLM 3 never invents a `PASS` without tool-produced evidence.

---

## Core Capabilities

### Intelligence

- 3-LLM architecture with explicit role separation
- Authoritative task state machine with typed states and validated transitions
- Planning, execution, independent review, and verification
- Replanning with failure evidence on verification failure
- Doom-loop detection (failure, strategy, and changed-file fingerprints)
- Bounded repair and replan limits
- Multi-agent verification pipeline (Tester, Reviewer, Security Reviewer)
- Loop engineering circuit breaker (stagnation detection, confidence tracking, budget enforcement)

### Context & Sessions

- Session isolation — each session maintains independent state, context, and history
- Context compaction — structured checkpoints preserve task state while reducing active context
- Persistent task state that survives application restart
- Crash recovery — inspects last operation state, never blindly replays side effects
- Workspace-based session ownership
- Session resume (`--resume`)

### Development

- File operations (read, write, create, delete)
- Shell command execution with permission gating
- Git integration (status, diff, log, branch, worktree mode)
- Test execution and result analysis
- Code analysis via SonarQube integration
- LSP-aware tooling
- MCP client (connect external tool servers)
- MCP server (expose AETHER memory to other tools)

### Extensibility

- Provider registry — register any OpenAI-compatible provider with multiple models (provider owns connection, model owns identity)
- Per-session role assignment — bind different providers/models per role per session
- Skills system — auto-discovered instruction files
- Plugin hooks (session start/end, agent spawn/complete)
- Custom subagent definitions via TOML
- MCP tool integration

### Security

- Hierarchical permission engine (read / edit / bash / delete / git_commit / network)
- Per-category allow / ask / deny policy
- Hard-denied dangerous commands (cannot be overridden)
- Workspace boundaries
- Credentials isolated per provider (env var or raw key, never in logs/events, fingerprint gates validation)
- Secret sanitization in analysis output

---

## Realtime Workspace Changes (OpenCode-style)

AETHER continuously detects and displays workspace changes **while the agent is working**, not only after the task finishes. The filesystem is the source of truth — not LLM claims.

```
Agent Tools → create/modify/delete/rename → Filesystem Watcher (notify, 280ms debounce, ignores .git/target/node_modules)
                                              ↓
                                        Git status --porcelain + diff --numstat HEAD
                                              ↓
                                        WorkspaceChanges { total_files, additions, deletions, files[] }
                                              ↓
                                        Tauri event workspace_changes_updated (workspace_id scoped)
                                              ↓
                                        Changes Panel (no refresh needed)
```

**What you see:**

```
Changes

3 files changed
+87  -24

M  src/openai.rs        +21 -4
M  src/gateway.rs       +42 -12
A  src/checkpoint.rs    +24
```

- `M` Modified, `A` Added, `D` Deleted, `R` Renamed, `U` Untracked — with accurate `+add -del` from Git.
- Clicking a file opens the existing diff viewer (`+` green, `-` red, `@@` hunk).
- Works for Git and non-Git workspaces; watcher degrades gracefully, never crashes the agent.
- Session-isolated: `Session A` changes never appear in `Session B`; switching workspaces restarts the watcher for the new workspace.

---

## Provider Hierarchy (Connection vs Model)

**Provider owns the connection. Model owns the identity. Session owns the assignment.**

```
Provider (id: nvidia)
├── Connection
│   ├── Protocol: openai_compatible
│   ├── Base URL: https://integrate.api.nvidia.com/v1  (once, not per model)
│   ├── Authentication: ( ) Env Var [NVIDIA_API_KEY]  ( ) API Key [••••]  ( ) None
│   └── Headers: X-Title: AETHER (optional, not model)
└── Models (many, no re-entering key)
    ├── nvidia/nemotron-3-nano-omni-30b-a3b-reasoning  (Display: Nemotron 3 Nano)
    ├── nvidia/llama-3.1-405b
    └── ...

Session selects:
  LLM 1 → nvidia / nemotron-3-nano  (Executor)
  LLM 2 → openrouter / claude-3.5   (Controller)
  LLM 3 → tokenrouter / gemini      (Reviewer)
```

- No duplication of Base URL or API key per model.
- Model ID vs Display Name are separate; API uses `model` field, UI shows display name.
- Validation is truthful: Base URL → Connectivity (`GET /models`) → Authentication (real bearer) → Model availability → API response (`POST /chat/completions` via same gateway as chat). `Can save` is not conflated with `health`.

---

## Long-Running Coding Tasks

AETHER is designed for engineering tasks that span multiple plan-execute-verify cycles. Task state persists across restarts, and context compaction allows long sessions to continue without losing important information.

### Task Lifecycle

```
Created → Understanding → Planning → Plan Ready → Executing
  → Reviewing → Verifying → Completed
  → (on failure) Replanning → Repairing → Reviewing → Verifying
```

Waiting states (`WAITING_USER`, `WAITING_TOOL`, `WAITING_NETWORK`, `BLOCKED`) survive application restart. Cancellation transitions safely to `CANCELLED` without corrupting persisted state.

### Context Compaction

When context approaches the model's window limit:

```
Context becomes large
       ↓
AETHER creates structured checkpoint
       ↓
Recent context retained
       ↓
Relevant state preserved
       ↓
Context rebuilt
       ↓
Task continues
```

The checkpoint preserves: objective, plan, completed work, active work, relevant files, important decisions, tool results, verification state, and next actions. Compaction never changes model assignments, role assignments, permissions, or session ownership.

Run `/compact` in the desktop app to trigger manual compaction, or let AETHER compact automatically when context approaches the safe threshold.

### Task State Machine

Every AETHER task is governed by an authoritative state machine:

- **16 typed states** — no arbitrary strings
- **Validated transitions** — illegal state changes are rejected
- **Transition history** — every state change records from/to state, active role, reason, and timestamp
- **Completion integrity** — `COMPLETED` requires verification evidence concluded by LLM 3
- **Repair limits** — configurable maximum repair attempts and replans
- **Doom-loop detection** — repeated failure/strategy/file fingerprints force replanning or failure
- **Crash recovery** — persisted state determines safe resume point

---

## See AETHER in Action

<p align="center">
  <img src="assets/aether.jpg" alt="AETHER desktop application" width="720" />
  <br><em>AETHER desktop — workspace-based sessions, Changes panel, and per-role model assignment</em>
</p>

---

## Installation

### Windows (recommended)

1. Download **`aether_<version>_x64-setup.exe`** from the [releases page](https://github.com/DhruvProgrammer/aether-code/releases/latest).
2. Run the installer → accept UAC → choose directory → Finish.
3. Launch **AETHER** from the Start Menu.

### Windows (portable)

Download **`aether-windows-x86_64.zip`**, extract, and run `aether.exe`. In a terminal it launches the TUI; with arguments it runs non-interactively.

### macOS

```bash
curl -L -o aether-macos-arm64.tar.gz \
  https://github.com/DhruvProgrammer/aether-code/releases/latest/download/aether-macos-arm64.tar.gz
tar -xzf aether-macos-arm64.tar.gz
sudo mv aether /usr/local/bin/
```

### Linux

```bash
curl -L -o aether-linux-x86_64.tar.gz \
  https://github.com/DhruvProgrammer/aether-code/releases/latest/download/aether-linux-x86_64.tar.gz
tar -xzf aether-linux-x86_64.tar.gz
sudo mv aether /usr/local/bin/
```

### Build from source

Requires Rust 1.75+ and a C linker.

```bash
git clone https://github.com/DhruvProgrammer/aether-code
cd aether-code
cargo build --release -p aether-cli
```

For the desktop app:

```bash
cargo install tauri-cli --version "^2.0" --locked
cargo tauri build --bundles nsis
```

---

## Quick Start

1. **Add providers.** Open Settings → LLM Providers → Add Provider. Use the radio for **Environment Variable** (e.g. `NVIDIA_API_KEY`) or **API Key** (raw `nvapi-...`) — they are distinct. Set Base URL once per provider, then add multiple models (e.g. `nvidia/nemotron-3-nano-omni-30b-a3b-reasoning`) without re-entering the key. `Check Connection` validates base URL + auth + `GET /models`.

2. **Set credentials securely:** For env var mode, `export NVIDIA_API_KEY="nvapi-..."` — frontend never receives the secret, health shows `env NVIDIA_API_KEY resolved`, not the value.

3. **Open a project folder** in the desktop app. The sidebar shows `Changes` in realtime and collapses smoothly (state persists).

4. **Assign models per session:** Click the model selector (bottom bar `— not set —` → LLM Configuration). Choose Provider/Model for LLM 1 (Executor) and LLM 2 (Controller); LLM 3 is optional. No routing — your choice is authoritative.

5. **Send `hello`:** Even with no model, invalid key, or offline network, the app shows a recoverable in-app error (`No model configured` / `Authentication failed` / `Network error`) and never crashes.

6. **Or run from terminal:**

```bash
aether "add a /login endpoint to the FastAPI app using JWTs and add tests"
```

---

## Models & Providers

### Hierarchy

AETHER enforces: `Provider { base_url, auth, headers }` → `Models[]` → `Session { LLM 1/2/3 → provider_id/model_id }`. Request resolves as `Session → Role → Provider ID → Model ID → Provider config → Secure credential → HTTP` (`crates/aether-gateway/src/config.rs:146`).

### Supported Endpoints

| Provider | Base URL |
|---|---|
| OpenAI | `https://api.openai.com/v1` |
| OpenRouter | `https://openrouter.ai/api/v1` |
| DeepSeek | `https://api.deepseek.com/v1` |
| NVIDIA NIM | `https://integrate.api.nvidia.com/v1` |
| Ollama | `http://localhost:11434/v1` |
| Any OpenAI-compatible | your endpoint |

### Model Gateway

All LLM calls pass through the gateway (`crates/aether-gateway/src/gateway.rs:144`):

- Explicit per-role bindings — no routing, no fallback
- Capability pre-checks (vision, tool calling, streaming)
- Typed failure classification (`invalid_api_key`, `rate_limited`, `model_not_found` — `crates/aether-gateway/src/error.rs:1`)
- Per-role provider isolation — one provider's failure never breaks another
- Fingerprinting (`crates/aether-gateway/src/fingerprint.rs:27` hashes `role|provider_id|base_url|model_id|api_key_env|headers|extra_body`) gates Save

---

## Configuration

Default paths: `~/.aether/config.toml`, `~/.aether/providers.json`, `~/.aether/sessions.db`, `~/.aether/workspaces.db`.

| Section | Purpose |
|---|---|
| `[agent]` | Model role bindings, max iterations, loop budget |
| `[models.<key>]` | Legacy: provider, base_url, model, api_key_env (migrated to providers.json) |
| `[providers.<id>]` | `display_name`, `protocol`, `base_url`, `auth_type`, `api_key`/`api_key_env`, `headers` |
| `[permissions]` | allow/ask/deny per category |
| `[memory]` | Embedding provider, retrieval settings |
| `[context]` | Max token budget |
| `[mcp.servers]` | External MCP tool servers |
| `[frontend]` | Visual review commands |
| `[appearance]` | Background, opacity, display mode |

Full annotated example: [`config.example.toml`](./config.example.toml)

---

## Architecture

```
AETHER
│
├── Desktop (Tauri 2) / CLI / TUI (shared run_task)
├── Workspace Manager (aether-workspace)
├── Session Manager (SQLite, aether-sessions)
├── Change Tracker (aether-changes: watcher + Git)
├── Task State Machine (aether-core/task_state)
├── Context Manager + Compaction (aether-context)
├── 3-LLM Orchestrator
│   ├── LLM 1 — Executor (tool-calling loop)
│   ├── LLM 2 — Planner (plan generation)
│   └── LLM 3 — Reviewer (verification + visual)
├── Multi-Agent Pipeline (Explorer, Tester, Reviewer, Security)
├── Loop Engine (circuit breaker)
├── Tool System (filesystem, shell, git, MCP, analysis)
├── Permission Engine (aether-permissions)
├── Model Gateway (explicit role bindings)
├── Evidence Engine (aether-evidence)
├── Persistent Memory (graph + vector + kv + skills, aether-mind)
├── Plugin System (aether-plugin)
└── Snapshot Manager
```

---

## Project Structure

```
aether/
├── crates/
│   ├── aether-core/        # Agent loop, executor, task state machine
│   ├── aether-cli/         # CLI binary + shared task runner
│   ├── aether-desktop/     # Tauri desktop app
│   ├── aether-gateway/     # Model gateway
│   ├── aether-config/      # Configuration + provider registry types
│   ├── aether-sessions/    # Session store (SQLite) + snapshots
│   ├── aether-context/     # Context management + compaction
│   ├── aether-changes/     # Realtime workspace changes
│   ├── aether-workspace/   # Workspace store
│   ├── aether-models/      # Provider adapters (OpenAI-compatible)
│   ├── aether-tools/       # Tool implementations + MCP client
│   ├── aether-permissions/ # Permission engine
│   ├── aether-mind/        # Persistent memory
│   ├── aether-evidence/    # Structured evidence engine
│   ├── aether-plugin/      # Plugin registry
│   ├── aether-analysis/    # SonarQube
│   └── aether-registry/    # Provider health checks
├── packages/app/           # Desktop frontend (TypeScript + Vite)
├── agents/                 # Subagent TOML definitions
├── skills/                 # Bundled skill files
└── assets/                 # Images
```

---

## Development

```bash
git clone https://github.com/DhruvProgrammer/aether-code
cd aether-code

# Build
cargo build --workspace

# Run tests
cargo test --workspace

# Frontend typecheck
cd packages/app && npx tsc --noEmit

# Frontend production build
npx vite build

# Desktop dev mode
cargo install tauri-cli --version "^2.0" --locked
cargo tauri dev --config crates/aether-desktop/tauri.conf.json
```

---

## Testing

- Task state transitions and completion integrity
- Session isolation and crash recovery
- Context compaction and serialization roundtrip
- Provider validation (fingerprint, error classification, secret redaction)
- Realtime changes: file create/modify/delete/rename, Git modified/untracked/deleted, non-Git fallback, watcher debouncing, session isolation
- Gateway behavior (no routing, role isolation)
- Sidebar collapse/persistence, chat error handling (no-model, invalid key, network, timeout)

```bash
cargo test --workspace   # all tests
cargo test -p aether-changes # realtime changes 10 tests
```

---

## Roadmap

### Implemented (0.22.0)

- 3-LLM architecture with explicit role binding and task state machine
- Model Gateway with provider isolation and fingerprinting
- Provider registry (provider owns connection, model owns identity) with env_var/raw distinction, headers, and migration
- Truthful validation (base URL, connectivity, auth, model, API response via same gateway)
- Realtime Git-aware Changes (watcher + diff, session-isolated, non-Git fallback)
- Workspace-based sessions with collapsible sidebar (state persists, smooth transition)
- Chat crash hardening (hello never crashes, all config/network errors are in-app)
- Context compaction, loop engineering, doom-loop detection, permission engine, MCP, SonarQube, desktop/CLI/TUI

### In Development

- Model discovery UI polish (fetch and add selected)
- Native Anthropic / Gemini adapters

### Planned

- VS Code extension
- Signed macOS `.dmg` and Linux `.deb`/`.rpm`
- Cross-project memory sharing (opt-in)

---

## FAQ

### What is AETHER?
An open-source autonomous AI coding agent in Rust with three specialized LLMs, persistent task state, and realtime workspace awareness.

### What is a multi-LLM coding agent?
One that separates planning, execution, and independent verification across models instead of one model self-grading. AETHER enforces `Planner ≠ Executor ≠ Reviewer`.

### Can it use local models?
Yes. Point any role at Ollama, LM Studio, vLLM, or llama.cpp — fully offline.

### Can it use different providers?
Yes. Example: `LLM 1 → NVIDIA / nemotron`, `LLM 2 → OpenRouter / claude`, `LLM 3 → TokenRouter / gemini` — per-session, no routing.

### How are changes tracked?
Filesystem watcher → debounce → `git status` + `diff --numstat` → `workspace_changes_updated` event → Changes panel `+add -del` per file, click to diff. Works while agent is running.

### What is context compaction?
Structured checkpoint (objective, plan, decisions, tool results) generated by LLM 2, then `system + checkpoint + recent tail` rebuilds context; history never deleted.

---

## Contributing

Bug reports, feature requests, and PRs welcome. Open an issue first for non-trivial changes.

```bash
git clone https://github.com/DhruvProgrammer/aether-code
cd aether-code
cargo test --workspace
cargo build --release -p aether-cli
```

The `agents/` directory contains per-agent TOML configs for customizing subagent behavior without touching Rust.

---

## License

MIT — see [LICENSE](./LICENSE).

```
AETHER © 2024-2026 aether contributors
Released under the MIT License.
```

---

<p align="center">
  <sub>Built with Rust · MIT licensed · No telemetry · No subscription · No lock-in</sub>
</p>
