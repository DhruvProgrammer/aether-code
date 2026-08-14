---
title: "AETHER — Build Plan"
status: "canonical"
audience: "engineering lead / agent"
summary: "Phase-gated plan to ship a Rust, OpenAI-compatible coding agent as a single Windows .exe, minimal memory-first."
---

# Build Plan — `aether`

**Goal:** a single, dependency-free Windows `.exe` coding agent that is **100% OpenAI API
compatible** (any `/v1/chat/completions` + `/v1/embeddings` endpoint), with an embedded
`aether-mind` memory engine, built in Rust for the RAM/footprint efficiency proven by jcode
and the clean TUI of grok-build.

**Non-goals (v1):** cloud multi-tenant, local LLM inference, LSP servers, MCP (post-MVP).

---

## 1. Why these references

| Reference | Borrowed idea |
|---|---|
| **opencode** | `AGENTS.md`/`CONTEXT.md` project-context model; `build`/`plan` agent split; ships a real Windows `.exe` |
| **jcode** | RAM efficiency target; OpenAI-*compatible* provider abstraction; embedded vector memory; passive memory recall |
| **grok-build** | Fullscreen minimalist TUI; composition-root binary crate; OpenAI-compatible transport |

We implement **independently** — no proprietary code is copied. The spec (§33) is explicit about this.

---

## 2. Provider strategy (OpenAI-compatible first)

One trait, many backends (spec §6). The binary ships with `OpenAICompatibleProvider` as the
**default and only required** path for v1:

```
chat:  POST {base_url}/chat/completions     (streaming + tool calls)
embed: POST {base_url}/embeddings
```

- `base_url`, `model`, `api_key_env` come from config; **never hardcoded** (spec §6).
- Works unmodified against: OpenAI, Azure OpenAI, OpenRouter, NVIDIA NIM, MiniMax, GLM,
  vLLM, LM Studio, Ollama (all expose the OpenAI shape).
- Secrets read from env or OS keychain; config stores only the *env var name*.

---

## 3. Phases (keep it runnable after each)

> Detailed crate map in [architecture.md](./architecture.md). Full phase list in [roadmap.md](./roadmap.md).

**Phase 1 — Minimal working agent (MUST compile + test before anything else)**
- `aether-cli` + `aether-core` + one provider (`OpenAICompatibleProvider`) + fs/terminal tools.
- Controller → Executor loop, `max_iterations` enforced. Ships a usable `.exe`.

**Phase 2 — Safety & sessions (✅ complete 2026-08-14)**
- Permissions engine (spec §14): `Policy::value_for(category)` + `check_bash` wired into `Executor::execute_tool`; dangerous bash always forces `Ask`.
- Planning mode (§13): `--plan` flag produces a read-only Policy (edit/delete/git_commit = Deny, bash = Ask) and injects a read-only directive.
- Git tools (§5): `git_status/diff/log/branch/checkout/add/commit` shell out to the `git` binary.
- SQLite sessions (§21): new `aether-sessions` crate records messages + tool calls + task/plan/result per session at `~/.aether/sessions.db`.

**Phase 3 — `aether-mind` MVP (✅ complete 2026-08-14)**
- New `aether-mind` crate (pure Rust, `redb`): graph (nodes + temporal edges), kv facts, and a
  brute-force cosine vector index (spec §9). **Deviation:** `usearch` (C++) is deferred — the
  vector store is pure-Rust so the GNU-only build needs no C++ linker; the `VectorStore` surface
  is unchanged for a later swap.
- Hybrid retrieval (`Mind::retrieve`) fuses vector + keyword + 1-hop graph (§9.7); wired into the
  Controller plan prompt via `Agent::run`, with repo `AGENTS.md`/`CONTEXT.md` discovery (§11, §12).
- Tools: `memory_save` / `memory_query` / `memory_forget` + `skill_search` (§10); `SkillIndex`
  discovers `SKILL.md` files by name+description only.
- LLM extraction pipeline (`extract::extract`, §9.3) is opt-in via `memory.auto_extract` (default off).
- Provider gained an `embeddings` method (`/v1/embeddings`).

**Phase 4 — Subagents (✅ complete 2026-08-14)**
- The `Executor` is now role-aware: a `system_prompt` override + `allowed_tools` allowlist let one
  tool-calling loop serve every role (spec §7).
- Roles in `aether-core::subagents`: `EXPLORER`, `REVIEWER`, `TESTER` (read-only by policy; Tester may
  run commands). `run_role()` builds a per-role `Policy` (read-only roles can't edit/delete/commit) and
  parses a structured `SubagentResult` JSON handoff (status/summary/findings/files).
- Orchestration in `Agent::run`: Controller plans → Coder (Executor) implements → optional Reviewer +
  Tester subagents run a handoff pass; their `SubagentResult` is appended to the outcome and recorded in
  the session. Planner = existing Controller; Researcher maps to Explorer (no web tool yet).
- Config: `[subagents]` with `enabled` (default false), `reviewer_model`, `tester_model`. Off by default,
  so existing single-agent behavior is unchanged.

**Phase 5 — Optimization & polish (✅ complete 2026-08-14)**
- **Context compaction (§20)**: `Executor` compacts the transcript each turn (keep system + first user +
  recent tail; truncate long tool outputs) bounded by `context.max_tokens`.
- **Cost routing (§8)**: `aether-core::router::select_model` chooses the Coder model by intent
  (cheap model for read-only/explanatory tasks via `agent.cheap_model`, capable for implementation
  tasks). The CLI now builds a provider per configured model and the Agent selects at run time.
- **Checkpoints / rollback (§15)**: `write_file` snapshots the before-state into the sessions store
  (`checkpoints` table); `aether --rollback <session>` restores the last checkpoint to disk.
- **Pantone minimalist UI**: `aether-cli/src/ui.rs` provides ANSI styling wired to `docs/design.md`
  tokens (Still Blue accent, Pavement ink, Cloud Grey muted, Marigold warn, Red Maple error). Banner,
  section, note, warn, error helpers. (Full `ratatui` TUI deferred — light ANSI styling keeps the
  binary dependency-free and build-safe on the GNU-only toolchain.)
- Observability via the sessions store (messages + tool calls + checkpoints + task/plan/result).

**Phase 6 — post-MVP research (✅ complete 2026-08-14)**
- **`aether-mind` as MCP server**: new `aether-mcp` binary speaks MCP JSON-RPC over stdio
  (initialize / tools/list / tools/call), exposing `memory_save` / `memory_query` /
  `memory_forget` / `skill_search`. Verified via piped requests.
- **MCP client**: `aether-tools::mcp` connects to external MCP servers over stdio, lists their
  tools, and adapts each into an `aether_tools::Tool` (category `network`). The CLI connects to
  configured `[[mcp.servers]]` at startup and registers their tools; failures are non-fatal.
  Verified: `aether` connected to `aether-mcp` and registered its tools.
- **Quantized vector index (§9)**: embeddings are scalar-quantized (f32 → i8 + per-vector scale)
  at store time and dequantized for cosine search — halves vector storage with negligible loss.
- **Local/cloud mode (§6)**: `--local` points all models at `agent.local_endpoint`
  (default `http://127.0.0.1:11434/v1`, i.e. Ollama/llama.cpp); cloud mode uses config base_urls.
  Any OpenAI-compatible local server works unchanged since the provider is already
  OpenAI-compatible.

---

## 4. Optimization guardrails (best code)

These keep the binary light (the differentiator vs OpenCode's 371 MB PSS):

1. **One memory-safe language.** Rust throughout; `unsafe` only in audited FFI wrappers (§17).
2. **No runtime deps at install.** Static `musl`/MSVC build → single `.exe`, no Node/Python.
3. **Lazy everything.** Skills, memory, web only loaded on demand (§9.5, §10, §12).
4. **Justify every crate** in `DEPENDENCIES.md`. Minimal high-quality stack only (§30).
5. **Mock providers in CI.** No live API calls; trait-based so the interface is real (§31).
6. **Per-crate tests.** `cargo test -p <crate>`; full-workspace builds are slow (grok-build lesson).

---

## 5. Exit criteria per phase
- Compiles on `stable` + `msvc` target.
- `cargo test` green for that crate set.
- `cargo clippy --all-targets` clean.
- Phase 1 `.exe` launches and completes one real task against a live OpenAI-compatible endpoint.

---

## See also
- [context.md](./context.md) — memory layers & context assembly
- [skills.md](./skills.md) — skill spec
- [architecture.md](./architecture.md) — workspace crate map
- [config.md](./config.md) — `config.toml` reference
- [roadmap.md](./roadmap.md) — release/`.exe` packaging
- [design.md](./design.md) — minimalist Pantone UI
