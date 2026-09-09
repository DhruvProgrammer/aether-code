# Reference Architecture Manifest

Local architectural reference cache for AETHER development. These files are
copied from upstream projects as study material; **AETHER is not derived from
them**. Implementation must be re-expressed in AETHER's own style and module
boundaries.

| Repository | Commit / Date (study snapshot) | Local Path |
|---|---|---|
| OpenCode (sst/opencode) | v0.21.0 era (2026-08) | `reference_architecture/opencode/` |
| Hermes Agent (NousResearch/hermes-agent) | main @ 2026-08 | `reference_architecture/hermes-agent/` |
| Grok Build (xai-org/grok-build) | main @ 2026-08 | `reference_architecture/grok-build/` |
| Fullstack Agent (jaredrhod/fullstack-agent) | main @ 2026-08 | `reference_architecture/fullstack-agent/` |

## File index by subsystem

### OpenCode (primary for: agent loop, sessions, context, compaction, provider/model, tools, workspace, diffs)

| File | Subsystem | Why relevant | AETHER target |
|---|---|---|---|
| `opencode/src/agent/agent.ts` | Agent loop, turn lifecycle, retries, cancellation | The reference implementation of a typed LLM turn with tool-calling and stream token | `crates/aether-core/src/agent_loop.rs` |
| `opencode/src/session/compaction.ts` | Session compaction (truncation, recent-tail budget, summarization) | The reference for "preserve original, recent tail, structured summary" | `crates/aether-context/src/checkpoint.rs`, `crates/aether-core/src/agent_loop.rs` |
| `opencode/src/session/overflow.ts` | Overflow detection (`usable = context - reserved_output`; compare **actual** provider usage vs `usable`) | The reference for percentage-based trigger and that overflow is *actual token* not *estimated* | `crates/aether-context/src/checkpoint.rs` (already has `should_compact` + `context_health`) |
| `opencode/src/session/processor.ts` | Session event handling, message persistence | Reference for typed `SessionCompactionEvent`, `SessionEvent` | `crates/aether-sessions/src/lib.rs`, `crates/aether-core/src/task_state.rs` |
| `opencode/src/provider/provider.ts` | Provider + Model schema (`Info`, `Model`, `Limit`, `Capabilities`, `cost`, `status`) | The reference for the full provider/model interface; AETHER must reach this depth | `crates/aether-gateway/src/`, `crates/aether-config/src/lib.rs` |
| `opencode/src/provider/transform.ts` | Provider transform (variants, reasoning transforms) | The reference for variants + model-specific tweaks | `crates/aether-gateway/src/transform.rs` (empty — needs real implementation) |
| `opencode/src/provider/auth.ts` | Provider auth scheme (oauth, api, env-var) | Reference for the `AuthConfig` enum and `resolve()` | `crates/aether-registry/src/provider.rs` |
| `opencode/src/tool/tool.ts` | Tool trait, schema, JSON validation | Reference for the canonical `Tool` interface | `crates/aether-tools/src/lib.rs` |
| `opencode/src/tool/registry.ts` | Tool registry | Reference for `ToolRegistry` | new `crates/aether-tools/src/registry.rs` (does not exist) |
| `opencode/src/config/config.ts` | Configuration | Reference for hierarchical config | `crates/aether-config/src/lib.rs` |
| `opencode/src/memory/memory.ts` | Memory abstraction | The shape of a `MemoryManager` interface | `crates/aether-mind/src/memory.rs` (already added) |

### Hermes Agent (primary for: persistent memory, prompt/context assembly, skills, long-running agent)

| File | Subsystem | Why relevant | AETHER target |
|---|---|---|---|
| `hermes-agent/agent/prompt_builder.py` | Prompt assembly (identity, skills, AGENTS.md context scanning) | The reference for the assembly pipeline with multiple sections; AETHER currently concatenates into one prompt | new `crates/aether-core/src/prompt.rs` (does not exist) |
| `hermes-agent/agent/memory_manager.py` | MemoryManager with provider registry, single external slot, prefetch_all / sync_all | The architectural reference for what we just added in AETHER v0.26.0 | `crates/aether-mind/src/memory.rs` (already done) |
| `hermes-agent/agent/memory_provider.py` | `MemoryProvider` trait | The trait shape we just adopted | same |
| `hermes-agent/agent/coding_context.py` | Coding context (project file sampling) | The reference for how to assemble code-aware context | new `crates/aether-tools/src/coding_context.rs` |
| `hermes-agent/agent/skill_utils.py` | Skill frontmatter parsing, environment matching | The reference for skill discovery and matching | `crates/aether-mind/src/skills.rs` |
| `hermes-agent/agent/skill_commands.py` | Skill runtime commands | The reference for skill lifecycle (CRUD) | same |
| `tools/registry.py` | Tool registry | The reference for `ToolRegistry` | new `crates/aether-tools/src/registry.rs` |
| `tools/toolsets.py` | Toolsets | The reference for grouped tool surfaces | new `crates/aether-tools/src/toolsets.rs` |

### Grok Build (primary for: Rust agent harness, runtime architecture, events, tool execution, TUI/runtime separation)

| File | Subsystem | Why relevant | AETHER target |
|---|---|---|---|
| `grok-build/crates/common/xai-grok-compaction/src/lib.rs` | Compaction core | The reference for an in-process compaction module | new `crates/aether-compaction-core/` (does not exist) |
| `grok-build/crates/common/xai-grok-compaction/src/config.rs` | Compaction config | The reference for config object | same |
| `grok-build/crates/common/xai-grok-compaction/src/prompt.rs` | Compaction prompt | The reference for the LLM-side prompt | same |
| `grok-build/crates/common/xai-grok-compaction/src/sampler.rs` | Compaction sample selection | The reference for choosing what to compact | same |
| `grok-build/crates/common/xai-grok-compaction/src/item.rs` | Compaction item | The reference for a content unit | same |
| `grok-build/crates/common/xai-grok-compaction/src/token.rs` | Token counting | The reference for token estimation | same / `crates/aether-context/src/checkpoint.rs` |
| `grok-build/crates/common/xai-grok-compaction/src/history/` | Compaction history | The reference for historical compaction | same |
| `grok-build/crates/common/xai-grok-compaction/src/intra_compaction/` | Intra-step compaction | The reference for in-loop compaction | same |
| `grok-build/crates/common/xai-grok-compaction/src/inter_compaction/` | Inter-step compaction | The reference for between-turn compaction | same |
| `grok-build/crates/common/xai-grok-compaction/src/code_compaction/` | Code-specific compaction | The reference for code-aware compaction | same |
| `grok-build/crates/common/xai-circuit-breaker/src/breaker.rs` | Circuit breaker | The reference for breaker state | new `crates/aether-circuit-breaker/` (does not exist) |
| `grok-build/crates/common/xai-tool-runtime/src/context.rs` | Tool runtime context | The reference for tool call context | `crates/aether-tools/src/lib.rs` |
| `grok-build/crates/common/xai-tool-runtime/src/dispatch.rs` | Tool dispatch | The reference for tool dispatch | same |
| `grok-build/crates/common/xai-tool-runtime/src/render.rs` | Tool output render | The reference for shaping tool output | same |
| `grok-build/crates/common/xai-tool-runtime/src/tool.rs` | Tool trait | The reference for tool trait | same |
| `grok-build/crates/common/xai-tool-runtime/src/streaming.rs` | Streaming tool result | The reference for streaming | same |

### Fullstack Agent (memory concepts)

| File | Subsystem | Why relevant | AETHER target |
|---|---|---|---|
| `CLAUDE.md` | Conductor script | Reference for the *installer* style of an agent | design notes only |
| `fullstack-agent.md` | Master prompt | Reference for the "agent as installer / runner" distinction | design notes only |
| `start.sh`, `start.bat` | Launchers | Reference for the agent launcher pattern | design notes only |

Fullstack is primarily a wrapper, not a coding agent. We copy only the
documents (not the empty repo skeleton) to inform our installer / agent
separation work. **Do not import** the wrapper code.

## What we did NOT copy (and why)

- **`opencode/node_modules/`** — dependencies.
- **`opencode/packages/*/node_modules/`** — dependencies.
- **`opencode/packages/opencode/.sst/`** — SST deployment infrastructure.
- **`opencode/packages/console/`**, **`opencode/packages/web/`**,
  **`opencode/packages/desktop/`** — unrelated web/desktop UIs.
  AETHER has its own Tauri desktop app and frontend.
- **`opencode/packages/function/`**, **`opencode/packages/slack/`** — unrelated integrations.
- **`hermes-agent/gateway/`**, **`hermes-agent/providers/`** — gateway/server
  infra. AETHER is a desktop app, not a hosted gateway.
- **`hermes-agent/.playwright-mcp/`**, **`hermes-agent/tests-js/`** — JS test
  infra for a different runtime.
- **`grok-build/bin/`**, **`grok-build/third_party/`** — proprietary build artifacts.
- **`grok-build/crates/common/xai-tracing/`, `xai-test-utils/`** — internal
  observability + test utilities. We will write our own telemetry / tests.
- **`grok-build/crates/common/xai-interjection-core/`** — proprietary
  Anthropic-specific code. Out of scope.
- **`grok-build/crates/common/xai-tool-protocol/`** — proprietary wire
  protocol. AETHER uses Tauri + JSON, not a custom protocol.
- **`grok-build/crates/common/xai-tool-types/`** — proprietary type system.
- **Fullstack Agent `assets/`, `bin/`, `start.*`** — wrapper scripts and
  binaries. Empty repo.
- **`.git/`, `node_modules/`, `dist/`, `build/`, `target/`, `coverage/`,
  `*.lockb`, generated `dist/`** in any subdir.

## Maintenance

When AETHER implements a subsystem and re-expresses it, the reference file
loses its "first-source-of-truth" status. Keep this manifest in sync with
the implementation index. If we add a sub-agent system in AETHER, the
relevant OpenCode / Grok file here should be re-read for the re-implementation
instead of being copied.
