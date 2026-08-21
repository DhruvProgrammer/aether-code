# AETHER

**An open-source, multi-LLM AI coding agent built around specialized planning, execution, and independent review.**

AETHER is an autonomous AI software engineering agent that separates the three hardest problems of AI-assisted coding into three dedicated LLM roles: one model understands and reviews, one model plans and orchestrates, and one model executes and builds. The result is a controlled Understand → Plan → Execute → Review → Verify engineering loop with persistent task state, context compaction, and long-running session support.

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

- Provider registry — register any OpenAI-compatible provider with multiple models
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
- API keys stored as environment variable references only — never in config files, logs, or LLM context
- Secret sanitization in analysis output

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
  <br><em>AETHER desktop — workspace-based sessions with per-role model assignment</em>
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

1. **Configure a model.** Open the desktop Settings tab and add a provider (any OpenAI-compatible endpoint), or create `~/.aether/config.toml`:

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

2. **Set your API key:**

```bash
export OPENAI_API_KEY="sk-..."
```

3. **Open a project folder** in the desktop app, or run from the terminal:

```bash
aether "add a /login endpoint to the FastAPI app using JWTs and add tests"
```

AETHER will understand the task, plan the implementation, execute it, review the result, and verify before reporting completion.

---

## Models & Providers

AETHER works with any OpenAI-compatible API endpoint. Each of the three roles is bound to a specific provider + model:

```
LLM 1 (Executor)  → configured Executor model
LLM 2 (Planner)   → configured Planner model
LLM 3 (Reviewer)  → configured Reviewer model (optional)
```

### Provider Registry

The desktop app includes a provider catalog where you register providers with their base URL, API key environment variable, and available models. Per-session role assignment lets you bind different models to different roles for each session.

### Supported Endpoints

| Provider | Base URL |
|---|---|
| OpenAI | `https://api.openai.com/v1` |
| OpenRouter | `https://openrouter.ai/api/v1` |
| DeepSeek | `https://api.deepseek.com/v1` |
| NVIDIA NIM | `https://integrate.api.nvidia.com/v1` |
| Ollama | `http://localhost:11434/v1` |
| LM Studio | `http://localhost:1234/v1` |
| vLLM | `http://localhost:8000/v1` |
| llama.cpp | `http://localhost:8080/v1` |
| Any OpenAI-compatible | your endpoint |

### Model Gateway

All LLM calls pass through AETHER's Model Gateway:

- Explicit per-role bindings — no routing, no fallback, no auto-switching
- Capability pre-checks (vision, tool calling, streaming)
- Typed failure classification (invalid key, rate limited, model not found, etc.)
- Per-role provider isolation — one provider's failure never breaks another role
- Live API validation with configuration fingerprinting

---

## Configuration

Default config path: `~/.aether/config.toml`. Override with `--config` or `AETHER_CONFIG`.

| Section | Purpose |
|---|---|
| `[agent]` | Model role bindings, max iterations, loop budget |
| `[models.<key>]` | Provider, base URL, model, API key env var |
| `[permissions]` | Per-category allow/ask/deny |
| `[memory]` | Embedding provider, retrieval settings |
| `[context]` | Max token budget for context management |
| `[mcp.servers]` | External MCP tool servers |
| `[frontend]` | Visual review capture/preview commands |
| `[appearance]` | Desktop background, opacity, display mode |

Full annotated example: [`config.example.toml`](./config.example.toml)

---

## Architecture

```
AETHER
│
├── Desktop (Tauri 2) / CLI / TUI
├── Workspace Manager
├── Session Manager (SQLite)
├── Task State Machine
├── Context Manager + Compaction
├── 3-LLM Orchestrator
│   ├── LLM 1 — Executor (tool-calling loop)
│   ├── LLM 2 — Planner (plan generation)
│   └── LLM 3 — Reviewer (verification + visual review)
├── Multi-Agent Pipeline (Explorer, Tester, Reviewer, Security)
├── Loop Engine (circuit breaker)
├── Tool System (filesystem, shell, git, MCP, analysis)
├── Permission Engine
├── Model Gateway (explicit role bindings)
├── Evidence Engine
├── Persistent Memory (graph + vector + kv + skills)
├── Plugin System
└── Snapshot Manager
```

The desktop app embeds the AETHER engine in-process — no subprocess, no visible console window, no PATH dependency.

---

## Project Structure

```
aether/
├── crates/
│   ├── aether-core/        # Agent loop, executor, controller, task state machine
│   ├── aether-cli/         # CLI binary + shared task runner
│   ├── aether-desktop/     # Tauri desktop app
│   ├── aether-gateway/     # Model gateway (role bindings, validation)
│   ├── aether-config/      # Configuration + provider registry types
│   ├── aether-sessions/    # Session store (SQLite) + snapshots
│   ├── aether-context/     # Context management + compaction
│   ├── aether-workspace/   # Workspace store
│   ├── aether-models/      # Provider adapters (OpenAI-compatible)
│   ├── aether-tools/       # Tool implementations + MCP client
│   ├── aether-permissions/ # Permission engine
│   ├── aether-mind/        # Persistent memory (graph + vector + skills)
│   ├── aether-evidence/    # Structured evidence engine
│   ├── aether-plugin/      # Plugin registry + hook bus
│   ├── aether-analysis/    # Code analysis (SonarQube)
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

AETHER uses automated tests across the workspace:

- Task state transitions (valid lifecycle, invalid transitions rejected)
- Completion integrity (no COMPLETED without evidence, only LLM 3 concludes)
- Session isolation (independent state per session)
- Repair and replan limits (bounded loops)
- Doom-loop detection (repeated fingerprint detection)
- Cancellation from every active state
- Crash recovery (safe resume from persisted state)
- Serialization roundtrip (state survives restart)
- Context compaction (checkpoint generation, atomic rebuild)
- Provider validation (fingerprint gating, error classification)
- Permission enforcement (policy checks, dangerous command denial)
- Gateway behavior (no routing, role isolation)

```bash
cargo test --workspace   # all tests
```

---

## Roadmap

### Implemented

- 3-LLM architecture with explicit role binding
- Authoritative task state machine with typed states
- Model Gateway (no routing, no fallback)
- Provider registry with per-session role assignment
- Workspace-based session management
- Context compaction with structured checkpoints
- Multi-agent verification pipeline
- Loop engineering circuit breaker
- Doom-loop detection
- Permission engine with hard denials
- Persistent memory (graph + vector + kv)
- MCP client + server
- Visual review loop (optional)
- SonarQube code analysis
- Desktop app (Tauri 2, in-process engine)
- CLI + TUI
- Session resume, rollback, snapshots
- Background mode, git worktree mode

### In Development

- Frontend task-state panel (real-time state display from backend)
- Extended crash recovery for mid-tool operations

### Planned

- VS Code extension
- Native Anthropic / Gemini adapters (no proxy required)
- Signed macOS `.dmg` installer
- Linux `.deb` / `.rpm` packages
- Cross-project memory sharing (opt-in)

---

## FAQ

### What is AETHER?

AETHER is an open-source AI coding agent built in Rust. It uses three specialized LLM roles — a planner, an executor, and an independent reviewer — to perform software engineering tasks with persistent state, bounded repair loops, and verification-driven completion.

### Is AETHER open source?

Yes. MIT-licensed, fully auditable source. No telemetry, no subscription, no cloud lock-in.

### What is a multi-LLM coding agent?

A coding agent that distributes responsibilities across multiple LLM roles rather than asking one model to plan, implement, and judge its own work. AETHER's three roles (Planner, Executor, Reviewer) are each bound to a user-configured model.

### How does AETHER's 3-LLM architecture work?

LLM 3 understands the task and reviews results. LLM 2 creates plans and replans on failure. LLM 1 implements changes and runs tools. The state machine coordinates handoffs between them. LLM 1 cannot declare completion; only LLM 3 can, with actual evidence.

### Can AETHER use local models?

Yes. Point any role at Ollama, LM Studio, vLLM, or llama.cpp. AETHER works fully offline with local endpoints.

### Can AETHER use different AI providers?

Yes. Each role can use a different provider and model. Register providers in the desktop Settings or in `config.toml`. Per-session role assignment lets you change bindings without editing config files.

### How does AETHER handle long coding sessions?

Task state persists in SQLite. Context compaction creates structured checkpoints that preserve objective, plan, decisions, and verification state while reducing active context. Sessions survive restarts via `--resume`.

### What is context compaction?

When the conversation approaches the model's context limit, AETHER generates a structured checkpoint (via LLM 2) capturing the essential task state, then rebuilds the active context as: system prompt + checkpoint + recent messages. The full history is never deleted.

### Does AETHER send my code anywhere?

Only to the API endpoints you configure. With a local model endpoint, nothing leaves your machine.

---

## Contributing

Bug reports, feature requests, and PRs are welcome. Open an issue first for non-trivial changes.

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
