# AETHER Gap Analysis

AETHER's current code size: **1.17 MB** of Rust + TypeScript + TOML/MD (excluding
build artifacts, target/, node_modules/, reference_architecture/, dist/, .gen/).

OpenCode's `packages/opencode/src/` alone is ~80 KLOC of TypeScript.
Grok Build is ~120 KLOC of Rust. Hermes is ~40 KLOC Python + extensive docs.

AETHER has a working foundation but is missing substantial architectural depth
in every major subsystem. This document compares each subsystem to the
references and identifies the gap.

## Inventory of what AETHER has today

A rough inventory (file:line ranges):

| Subsystem | Current files | Lines (approx) |
|---|---|---|
| Provider / model config | `crates/aether-config/src/lib.rs`, `crates/aether-gateway/src/{config.rs, gateway.rs, validate.rs, store.rs, fingerprint.rs}`, `crates/aether-registry/src/provider.rs`, `crates/aether-models/src/openai.rs`, `crates/aether-desktop/src/main.rs::providers_list / providers_save / providers_validate` | ~3000 |
| Tools | `crates/aether-tools/src/{lib.rs, git.rs, mcp.rs, analysis.rs}` | ~900 |
| Sessions | `crates/aether-sessions/src/lib.rs` | ~600 |
| Context / compaction | `crates/aether-context/src/checkpoint.rs`, `crates/aether-core/src/compaction_store.rs` | ~1000 |
| Memory | `crates/aether-mind/src/{lib.rs, memory.rs, tools.rs, skills.rs, vector.rs, extract.rs, context.rs}` | ~2000 |
| Agent loop | `crates/aether-core/src/agent_loop.rs`, `executor.rs`, `task_state.rs`, `controller.rs`, `subagents.rs` | ~3000 |
| Realtime changes | `crates/aether-changes/src/lib.rs` | ~600 |
| Frontend | `packages/app/src/{main.ts, api.ts}` | ~3000 |
| Desktop | `crates/aether-desktop/src/main.rs` | ~2000 |
| **Total source** | | **~1.17 MB** |

## Subsystem-by-subsystem analysis

### 1. Agent loop

**AETHER today** — `crates/aether-core/src/agent_loop.rs:258` `Agent::run()`:
a single big async function. Phase markers exist, but the loop is a 1000-line
method with an inline `for iter in 0..loop_budget` doing plan → executor →
sub-agents → verify → decide in one function. Sub-agents share the executor's
coder pattern (`correction-coder`). Cancellation via `Notify`. Context budget
hard-coded into Agent fields; no per-turn state. No explicit `Turn` object.

**OpenCode** — `agent/agent.ts`: small LoopEngine that orchestrates
compaction, model call, and tool execution. Stream responses are typed and
piped into the message store immediately. Cancellation clean. State machine
explicit (idle, creating, running, done, error).

**Grok Build** — `xai-circuit-breaker`, `xai-grok-compaction` crate
separation: each responsibility is a dedicated crate. Compaction is a
"step" inside the loop. The breaker's state is observable and reset on
agent rebuild.

**Hermes** — `run_agent.py`: explicit `KanbanCard` lifecycle, `mavis_kb`
init step, persistence fan-out, tool concurrency limits.

**Gap**:
- Single-megafunction `Agent::run` — hard to test, hard to compose.
- No `Turn` object — state mutations are implicit in local variables.
- No explicit agent state machine — phases are `for` loops with `eng.note_*` calls.
- Cancellation works but only at outer loop boundaries.
- No per-agent concurrency / rate limiting.

**Target**:
- Extract `AgentLoop` with a `Turn` object (`RunLoop` / `PlannerTurn` / `ExecutorTurn` / `VerifierTurn`).
- Extract a `ContextLoop` for compaction, a `ToolLoop` for tool execution.
- Add `AgentLoopState` enum (`Idle`, `Planning`, `Executing`, `Verifying`, `Compacting`, `Complete`, `Failed`, `Cancelled`).
- Add a `tools_run_in_parallel` map with cancellation propagation.
- Add observable per-turn metrics (latency, tokens, cost).

### 2. Context manager

**AETHER today** — `crates/aether-context/src/checkpoint.rs:1`:
`CompactionCheckpoint` struct, `SessionCompactor` with `compact()` and
`rebuild_context()`. Heuristic-based token estimation (chars/4). Reuses the
executor. `executor.rs:208` `preflight_compact` runs the compactor on every
model call. The session-level `ContextHealth` (Safe/Warning/Critical) was
added in v0.23 at 70%/85% thresholds (grok-build style).

**OpenCode** — `session/compaction.ts`: turn-based selection, recent-tail
budget = `min(15k, max(2k, 25% usable))`. Tool output truncated to 2k chars
before summarization. `usable` = `model.limit.context - reserved_output` where
`reserved_output` = `min(COMPACTION_BUFFER=20k, maxOutputTokens)`.
Overflow = `tokens.total >= usable`, where `tokens.total` is the **actual
provider-reported usage** (not an estimate).

**Grok Build** — `xai-grok-compaction`: three modes
(`FullReplace`/`StepsOnly`/`HistoryOnly`/`HistoryThenSteps`). Errors are
a non-fatal enum (`NothingToCompact`, `Timeout`, `EmptyResponse`,
`InsufficientReduction`).

**Hermes** — `coding_context.py`: project-aware context sampling; AGENTS.md /
.hermes.md / SOUL.md discovery with threat-pattern scanning.

**Gap**:
- Token estimation is heuristic (chars/4). No actual provider usage
  is consumed by overflow detection.
- Recent-tail budget is hardcoded in `compact_messages` (4 messages).
- No tool-output truncation (2k char cap) before compaction.
- No inter-step vs intra-step distinction (Grok has three modes).
- No file-context sampler (Hermes `coding_context.py`).
- No threat-pattern scanning on AGENTS.md content (Hermes).
- No per-agent compaction configuration.

**Target**:
- Add a `TokenBudget` struct (system + role + tools + workspace + memory +
  conversation + tool results + checkpoint + recent + requested output).
- Add `ToolOutput::Truncated` wrapping tool results; `compact_messages` reads
  only the truncated form.
- Split `Compaction` into `intra` (within a step) and `inter` (between steps)
  with shared core.
- Add `IntraCompactionResult` + `InterCompactionResult` carrying
  `tokens_before/after`, `turns_compacted`, `elapsed`, `summary`.
- Add file-context sampler: pick N files by importance (recent edits, hot
  symbols); include bounded summaries.
- Add prompt-injection scanner for `AGENTS.md` / `CLAUDE.md` / `.hermes.md`.

### 3. Memory

**AETHER today** — `crates/aether-mind/src/memory.rs`: `MemoryManager` with
`MemoryProvider` trait, single external slot enforcement,
`prefetch_all(query)` / `sync_all(turn)` orchestration. `BuiltinMemoryProvider`
wraps `Mind` graph+vector+kv store.

**OpenCode** — `provider/provider.ts:1076+`: memory is part of provider
config; persistence fan-out per provider.

**Hermes** — `memory_manager.py:443+`: `add_provider` blocks second external,
tool-name shadowing check (`_HERMES_CORE_TOOLS`). `build_system_prompt()`,
`prefetch_all(query)`, `sync_all(turn)`. `MemoryProvider` interface
`get_tool_schemas()` + `system_prompt_block()`. `prefetch_all` is
synchronous; `sync_all` is best-effort.

**Gap**:
- AETHER's `MemoryProvider` has `prefetch` returning a single string; Hermes
  supports per-provider `get_tool_schemas()` (memory can expose tools).
- AETHER's `BuiltinMemoryProvider` doesn't actually use `Mind` — it returns
  empty `prefetch`. Should wire graph+vector+kv recall.
- No memory tool surface (search/save/delete).
- No per-provider `system_prompt_block()` (currently only one block,
  joined).
- `prefetch_all` is synchronous in both — fine. But no caching, no rate
  limit, no async.
- No memory stats / observability.

**Target**:
- Wire `BuiltinMemoryProvider::prefetch` to call `Mind::get_node` /
  `vector::query` and return a short markdown summary.
- Add memory tools: `memory_search(query)`, `memory_save(key, value)`,
  `memory_forget(id)`.
- Add a `MemoryStats` struct exposed for diagnostics.
- Add `get_tool_schemas()` to the trait; built-in returns `[]` (memory
  surface is the `Mind` instance, not a tool).

### 4. Tool registry

**AETHER today** — `crates/aether-tools/src/lib.rs`: a `Tool` trait, a
`default_tools()` factory. No `ToolRegistry` per se. Each tool is hand-wired
into `default_tools()` plus `analysis_tools()`, `git_tools()`, `mcp_tools()`,
`memory_tools()`, `skill_tools()`.

**OpenCode** — `tool/registry.ts`: full registry with IDs, permissions,
MIME types, status (active/disabled). Plugin integration.

**Hermes** — `tools/registry.py`, `tools/toolsets.py`: tool registry with
core tool names reserved, toolsets that group tools for contexts, capability
declarations.

**Gap**:
- No `ToolRegistry` class — all tools are ad-hoc.
- No `Toolset` concept — can't scope tools per agent role.
- No per-tool permission discovery on registration.
- `Tool` trait lacks `required_capability`, `result_schema`, `streaming`.
- Tools not pluggable; cannot load from a directory.

**Target**:
- `ToolRegistry` with `register(tool)`, `unregister(name)`, `get(name)`, `names()`,
  `for_role(role)` (toolsets).
- Toolsets: e.g. `planner_tools`, `executor_tools`, `reviewer_tools`.
- Plugin loading: `load_from_dir(path)` — read `SKILL.md` or `plugin.toml`.

### 5. Workspace

**AETHER today** — `crates/aether-tools/src/lib.rs` (read_file, write_file,
list_directory, grep, execute_command, git tools). `read_file` now supports
offset/limit + binary guard + size caps (v0.26). `grep` has sandbox check +
binary skip. `list_directory` has size + sandbox + cap.

**OpenCode** — `tool/tool.ts` (file ops) and a workspace abstraction that
lives behind a trait so the runtime can swap the workspace implementation
(local FS, sandboxed FS, remote, etc.).

**Hermes** — `coding_context.py` does repo-aware file sampling.

**Gap**:
- No `Workspace` trait — file ops are tied to `std::path`.
- No file metadata beyond `size + type` (no mtime, no perms, no hash).
- No `search_content` (regex) — only literal grep.
- No `replace_in_file` with safety checks (string_match_globally).

**Target**:
- Introduce a `Workspace` trait with `read`, `list`, `search`, `stat`,
  `replace`.
- Add `stat()` returning `FileMeta { size, mtime, readonly, is_binary, mime }`.
- Add `replace_in_file(path, old, new, all_occurrences)` with string_match
  safety.

### 6. Realtime changes

**AETHER today** — `crates/aether-changes/src/lib.rs`: full implementation
with `notify` watcher, 280ms debounce, Git-aware diff via `git status
--porcelain` + `diff --numstat HEAD`, non-Git fallback. 10 tests pass.

**OpenCode** — `session/processor.ts` and `tool/` events drive the diff.

**Gap** — this subsystem is well-built. Possibly:
- Add a "scope" concept (only watch `<workspace>/src/**`).
- Add a richer event for file rename (currently reported as Created+Deleted).
- Add a "session-scoped" diff overlay (e.g. "only changes since
  session-start" — useful for compact diffs).

### 7. Provider / model architecture

**AETHER today** — `crates/aether-config/src/lib.rs` has `ProviderEntry` +
`ModelEntry`. `crates/aether-gateway/src/` has the gateway + validation.
`crates/aether-models/src/openai.rs` is the only provider. The
`aether-gateway/src/validate.rs` validates one model at a time. Capabilities
are partly modeled (`tool_calling`, `vision`, `streaming`).

**OpenCode** — `provider/provider.ts:1048+` defines the full
`Model` interface with `id`, `providerID`, `api {id, url, npm}`, `status`,
`headers`, `options`, `cost {input, output, cache, tiers,
experimentalOver200K}`, `limit {context, input, output}`,
`capabilities {temperature, reasoning, attachment, toolcall, input, output,
interleaved}`. `Info` adds `name`, `env` (array of env var names), `key`.

**Gap**:
- AETHER's `ModelEntry` lacks `cost`, `status`, `family`, `release_date`,
  `variants`, `interleaved` fields.
- AETHER's `ProviderEntry` lacks `cost`, `tiers`, `interleaved`, `extra_env`.
- AETHER's `auth` is a flat `api_key_env` string. OpenCode has OAuth +
  env + raw + custom-header.
- AETHER's `headers` is a JSON `Value` blob. OpenCode has typed headers.
- No provider variants (e.g. `gpt-5-mini` + `gpt-5-mini-responses` for
  Azure-style variants).
- No status (active/alpha/deprecated) on the AETHER model side.
- The `aether-gateway` has a `fingerprint_binding` for save-gating; the
  flow is good but the model is bare.

**Target**:
- Extend `ModelEntry` with: `cost` (input/output/cache), `family`,
  `release_date`, `status` (active/alpha/deprecated), `variants` map,
  `input/output` modality flags.
- Extend `ProviderEntry` with: `display_name`, `capabilities` (rate limit,
  tool call), `extra_env`, `protocol_version`.
- Convert `auth` to enum: `BearerEnv { name }`, `BearerRaw { key }`,
  `ApiKeyEnv { name }`, `ApiKeyRaw { key }`, `None`. (This is done at
  `aether-registry/src/provider.rs:8-29`; mirror it in `aether-config`.)
- Add `ProviderRegistry` with `register`, `get`, `all`, `for_role`.

### 8. Event bus

**AETHER today** — `crates/aether-core/src/task_state.rs` has
`TaskEventKind` enum with `TaskStateChanged`, `ToolStarted`, `ToolCompleted`,
`FileCreated/Modified/Deleted`, `ContextWarning`, `CompactionStarted/
Completed/Failed`, `VerificationStarted/Completed`, etc. The
`Executor::emit_tool` method (v0.23) emits per-tool events. The
desktop bridges via `task-state` Tauri event.

**OpenCode** — extensive event bus with `EventV2`, `EventV2Bridge`,
`SessionEvent`, `SessionCompactionEvent`, `ProviderEvent`, `ToolEvent`.
Each event is a typed effect.

**Gap**:
- The `TaskEventKind` enum is in `task_state.rs` which lives in
  `aether-core`. Good. But the agent loop only emits a subset; many
  events are defined but never produced.
- The desktop's `task-state` listener re-emits everything as one blob
  to the frontend; the frontend can't filter.
- No per-tool event object (we emit only tool name + operation preview,
  not the full tool invocation context).
- No per-message event (only at the lifecycle boundaries).

**Target**:
- Add a `RuntimeEventBus` struct that owns typed channels per event
  category (model, tool, file, context, verification, session).
- Add `bus.publish(Event)`; runtime components subscribe by category.
- Add a "filter" parameter to the desktop's task-state Tauri event so
  the frontend can subscribe to just `Tool*` or `File*` events.

### 9. Verification system

**AETHER today** — `crates/aether-core/src/task_state.rs`: `VerificationEvidence`
struct with `EvidenceItem { kind, status, detail, output_ref }`. Built up
in `agent_loop.rs` from the tester/reviewer/security sub-agents' `SubagentResult`.
v0.23 added a `parse_test_counts` parser that catches `27 passed / 2 failed`
text from tool output. `VerificationStarted/Completed` events exist.

**Gap**:
- No persistence of verification evidence per session (it's in-memory).
- No "re-verify" command that runs tests after a fix.
- No diff between current verification and last-known-good.
- No "verify this file" tool.

**Target**:
- Persist `VerificationEvidence` to `aether-sessions`.
- Add `verify_now(workspace, check_type)` command.
- Add diff view: last-pass vs current.

### 10. Session persistence / recovery

**AETHER today** — `crates/aether-sessions/src/lib.rs`: SQLite-backed
session store with messages, kv, traces, checkpoints (via
`aether-core::compaction_store`). Session IDs persist; tool calls persist.
`recovery_state()` on the state machine gives a safe resume point.

**OpenCode** — sessions are SQL-backed, with full message history
storage and per-part granularity (text / tool / file / reasoning).

**Gap**:
- AETHER's session messages are flat strings, not part-typed.
- No message versioning; compaction appends a `compaction` part on
  user side. OpenCode interleaves parts.
- No crash recovery on the agent loop — `recovery_state` exists in the
  state machine but the agent loop doesn't actually use it to skip ahead.

**Target**:
- Add `MessagePart` enum (Text, Tool, File, Reasoning, Compaction).
- Wire `recovery_state` to actually truncate the conversation at the
  recovered step.

### 11. Prompt assembly

**AETHER today** — `crates/aether-core/src/agent_loop.rs:22`:
`AETHER_CORE_SYSTEM_PROMPT`, `KARPATHY_POLICY`. Per-role: `PLANNER_SYSTEM`,
`CODER_SYSTEM`. Per-agent: `TESTER_SYSTEM`, `EXPLORER_SYSTEM`, etc.
The prompt is just a `format!` concatenation in `run_role`.

**Hermes** — `prompt_builder.py` is a 1000+ line module that explicitly
assembles: system identity, platform hints, skills index, context files
(AGENTS.md, .hermes.md) with git-root walking and threat-pattern
scanning, memory, plus role overlay. Stage-based:
`_build_system_prompt()` calls sub-builders.

**Gap**:
- AETHER's prompt is monolithic `format!` — no assembly pipeline.
- No skills index in the prompt (skill descriptions are in
  `aether-mind::skills` but never summarized into the system prompt).
- No platform hints (working dir, OS, env).
- No project instruction scanning (AGENTS.md / CLAUDE.md).
- No threat-pattern scanning on context files (Hermes has
  `tools/threat_patterns.py` with a `context` scope).

**Target**:
- Build `crates/aether-core/src/prompt.rs` with `PromptBuilder`.
- Add skills index injection.
- Add platform-hint block.
- Add context-file scanning (AGENTS.md, .aether.md, etc.) with
  threat-pattern detection (reused in workspace layer).

### 12. Security

**AETHER today** — `crates/aether-permissions/`: policy enforcer (read/
edit/bash/delete/git_commit/network) with `Global → Project → Role →
Agent → Tool` hierarchy. Tools have `required_permission` field. Filesystem
sandbox check is in `crates/aether-tools`. `crates/aether-config` stores
env-var names (not raw keys); raw keys in `ProviderEntry.api_key` are
possible but not logged. `aether-gateway/src/validate.rs` masks secrets in
error messages.

**Gap**:
- The threat-pattern scanner mentioned above is missing — context files
  (AGENTS.md, README.md) are injected into the prompt verbatim.
- The `read_file` tool reads any path under the workspace boundary; need
  per-file size cap to prevent huge-file context pollution.
- Tool output is sent to the model verbatim — need max-bytes cap.
- The Permission engine has no "approval audit" — once an action is
  approved, no record persists.

**Target**:
- Add a `ThreatScanner` over file content (basic patterns: "ignore all
  previous instructions", "send the key to", etc.).
- Enforce per-tool output size cap (e.g. 50 KB max output to model).
- Add `Workspace::size_cap()` for `read_file` (already partial).

### 13. Tests

**AETHER today**:
- `aether-core` tests: 67 (state machine, model assignments, etc.)
- `aether-gateway` tests: 45
- `aether-mind` tests: 9
- `aether-tools` tests: 8
- `aether-changes` tests: 10
- **Total unit tests: ~140**

OpenCode: hundreds of tests per package. Hermes: large integration
test suite. Grok Build: per-subsystem test files.

**Gap**:
- Integration tests for the full agent loop (real codepath).
- Fixtures (fake providers, fake tools, fake workspaces) — currently all
  tests use the real network or real filesystem.
- Long-running tests (a 5-minute task) — none.
- Test scenarios for the three-LLM coordination (LLM1 calls LLM2, LLM3
  reviews LLM1) — none.
- Test scenarios for provider/credential migration — partial.
- Test scenarios for compaction / context health — partial.
- Test scenarios for permission boundary enforcement — partial.

**Target**:
- Add `MockProvider` implementing `ModelProvider` with deterministic
  responses for testing.
- Add `MockWorkspace` for sandbox-free filesystem tests.
- Add `crates/aether-core/tests/agent_loop.rs` integration test.
- Add a "long running" test that runs a 5-turn task.

## What we will implement in v0.27 (this turn)

| Phase | Subsystem | What we add |
|---|---|---|
| 2 | Provider / Model | Extend `ModelEntry` with `cost`, `family`, `status`, `release_date`, `variants`; add `ProviderRegistry` |
| 4 | Tools | Add `ToolRegistry` + `Toolset` concept + `Workspace` trait |
| 5 | Workspace | Add `Workspace` trait, `stat`, `replace_in_file`, `search_content` |
| 6 | Event bus | `RuntimeEventBus` with typed per-category channels + filtering |
| 8 | Context | Add `TokenBudget`, tool-output truncation, file-context sampler |
| 9 | Compaction | Split into intra/inter, add `CompactionResult` types |
| 11 | Session | Add `MessagePart` enum, wire crash recovery |
| 14 | Tests | Add `MockProvider` + `MockWorkspace` + integration test |
| 15 | Prompt | Build `PromptBuilder` with skills + platform + threat-scan |

We will not implement in this turn (deferred to keep scope manageable):

- Full OAuth flows (only env-var + raw-key for now)
- Provider plugins loaded from directory
- Multi-agent concurrency (only main agent + sub-agents)
- External memory providers (only built-in)

We will keep all of the above in mind for subsequent versions.
