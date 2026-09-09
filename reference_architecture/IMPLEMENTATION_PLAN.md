# Implementation Plan — v0.27+

This plan tracks what we implement in this turn (build mode) to grow
AETHER toward OpenCode / Hermes / Grok-Build level architectural maturity.

## Phase 0 — Reference cache

- Created `reference_architecture/{opencode,hermes-agent,grok-build,fullstack-agent}/`
  with curated source files (not full repos) and `REFERENCE_MANIFEST.md`
  + `AETHER_GAP_ANALYSIS.md`.

## Phase 1 — Provider / model depth (`aether-config`, `aether-gateway`)

- Extend `ModelEntry` with `cost {input, output, cache {read, write},
  tiers, experimentalOver200K}`, `family`, `status` (active / alpha /
  deprecated), `release_date`, `variants` (id -> variant), `input`
  (text/audio/image/video/pdf), `output` (text/audio/image).
- Extend `ProviderEntry` with `capabilities` (rate limit / tool support),
  `extra_env`, `protocol_version`.
- Add `aether-gateway/src/registry.rs` (`ProviderRegistry`): `register`,
  `get`, `all`, `for_role(role)`, `for_model(provider_id, model_id)`.
- Add tests in `aether-gateway/tests/registry.rs` (10+ tests).

## Phase 2 — Tool registry (`aether-tools`)

- Add `aether-tools/src/registry.rs` (`ToolRegistry`): `register`,
  `unregister`, `get`, `names`, `for_role(role)`.
- Add `aether-tools/src/toolsets.rs` (`Toolset`): `Planner`, `Executor`,
  `Reviewer` — each is a `Vec<String>` of tool names; the agent loop
  picks the right toolset per LLM role.
- Add `MockTool` for tests.
- Tests: 5+ for the registry.

## Phase 3 — Workspace trait (`aether-tools`)

- Add `aether-tools/src/workspace.rs` (`Workspace` trait) with `read`,
  `read_range`, `list`, `stat`, `search`, `replace`.
- Implement `LocalWorkspace` (current `read_file`/`write_file`/etc.
  backed by `std::path`).
- Add `replace_in_file(path, old, new, all_occurrences)` with safety
  checks.
- Add `stat(path) -> FileMeta { size, mtime, is_binary, mime }`.
- Tests: 5+ for the trait + LocalWorkspace.

## Phase 4 — Event bus (`aether-core`)

- Add `aether-core/src/event_bus.rs` (`RuntimeEventBus`): per-category
  channels (model, tool, file, context, verification, session).
- Each `EventKind` already exists; group them by category.
- Add `bus.publish(category, payload)`, `bus.subscribe(category) -> Receiver<Payload>`.
- The executor emits via the bus (currently uses `task_event_sink` — keep
  the sink for backward compat, also publish to the bus).
- Tests: 5+ for the bus (routing, isolation, filter).

## Phase 5 — Context manager (`aether-context`)

- Add `aether-context/src/budget.rs` (`TokenBudget`) with components
  (system, role, skills, tools, workspace, memory, conversation, tool
  results, checkpoint, recent, requested output) and `total()`.
- Update `estimate_request_tokens` to use the new `TokenBudget`.
- Add tool-output truncation (2 KB default) in `compact_messages` and
  the new `Compactor` (so 4 KB tool output becomes 2 KB before
  summarization).
- Tests: 3+.

## Phase 6 — Compaction split (`aether-context`)

- Add `aether-context/src/compaction/result.rs` (`IntraCompactionResult`,
  `InterCompactionResult`) carrying `tokens_before/after`,
  `turns_compacted`, `elapsed`, `summary`, `error` enum
  (`NothingToCompact`, `Timeout`, `EmptyResponse`,
  `InsufficientReduction`).
- Add `aether-context/src/compaction/intra.rs` and `inter.rs` modules
  that share a core `compact_turns` function.
- `SessionCompactor::compact` now returns the typed result.
- Tests: 5+ for the split.

## Phase 7 — Session parts (`aether-sessions`)

- Add `MessagePart` enum (Text, ToolCall, ToolResult, FileRef,
  Reasoning, Compaction).
- Add a migration on `SessionStore::open` that splits the existing flat
  `content` string into a default `MessagePart::Text`.
- Wire `recovery_state` to actually truncate the conversation at the
  recovered step.
- Tests: 3+.

## Phase 8 — Prompt builder (`aether-core`)

- Add `aether-core/src/prompt.rs` (`PromptBuilder`).
- Stages: identity → role → platform hint (working dir, OS) → skills
  index → context files (AGENTS.md, .aether.md, CLAUDE.md) with
  threat-pattern detection → memory context → checkpoint → tool
  schema → request.
- Add `aether-core/src/threat.rs` with basic patterns
  (Hermes-style — "ignore all previous", "send the key", etc.).
- Replace `format!` concatenation in `agent_loop.rs:run` and
  `run_role` with `PromptBuilder`.
- Tests: 5+.

## Phase 9 — Test infra

- Add `aether-core/src/testing.rs` (`MockProvider`, `MockWorkspace`).
- Add `crates/aether-core/tests/agent_loop.rs` — an integration test
  that runs the full loop end-to-end against `MockProvider` + `MockWorkspace`,
  asserts that the state machine transitions to `Completed`, that
  typed events are emitted in order, and that compaction fires when
  estimated tokens cross 80% of the model window.
- Add `aether-core/tests/runtime_event_bus.rs` — 5+ tests.

## Phase 10 — Wire all phases into `agent_loop.rs`

- Replace `Agent::run` with calls into the new subsystems (bus, prompt
  builder, tool registry, workspace trait, etc.).
- Keep the existing typed events working.

## Phase 11 — Verify and ship

- `cargo check --workspace` pass
- `cargo test --workspace` pass
- `npx tsc --noEmit` pass
- `npx vite build` pass
- Long-running test: run a 5-turn `MockProvider` script that
  simulates file editing + test failure + fix; verify LLM 1 → LLM 3 →
  LLM 1 chain works.
- Bump `0.26.0` → `0.27.0`, `tauri.conf.json` version, commit, push, tag
  `v0.27.0`, `gh release create`.

## What is OUT of scope (explicitly deferred)

- Provider OAuth flows
- External memory providers (only the trait + built-in is built)
- Provider directory plugins
- Multi-agent concurrency / rate limiting
- Provider directory plugin loader
- WebSocket / TUI mode (Grok Build style)
- Tauri v2 capability review (already in place; this PR won't change it)
- OpenCode-style split brain fix (already in v0.24 / v0.25)
- Memory `get_tool_schemas` (deferred to v0.28)

## Risk

- The agent loop is the highest-risk file to modify. We will refactor
  in small steps: extract `build_prompt`, extract `turn_executor`,
  extract `turn_verifier`. Each step must keep `cargo check` green.
- `CompactionCheckpoint` JSON format is on disk; do not break it.
- Tauri v2 events: keep the same wire names so the frontend still works.

## Order of execution within v0.27

1. `Workspace` trait (Phase 3) — isolated.
2. `MessagePart` (Phase 7) — isolated; migration is safe.
3. `TokenBudget` + truncation (Phase 5) — additive; doesn't break `compact_messages`.
4. `ProviderRegistry` (Phase 1) — additive; existing `GatewayBundle`
   still works.
5. `ToolRegistry` + `Toolset` (Phase 2) — additive; existing
   `default_tools()` still works.
6. `RuntimeEventBus` (Phase 4) — additive; existing `task_event_sink`
   still works.
7. `PromptBuilder` (Phase 8) — replace `format!` calls in agent_loop
   *carefully* with the new builder.
8. `CompactionResult` split (Phase 6) — additive; existing
   `SessionCompactor` API gets a richer return type via a new method
   while old one stays.
9. Mock test infra + integration test (Phase 9).
10. Wire all into `agent_loop.rs` (Phase 10).
11. Verify, commit, tag, push, release.
