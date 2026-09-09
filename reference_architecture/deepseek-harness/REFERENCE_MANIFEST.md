# DeepSeek Harness — Reference Manifest

Reference: https://github.com/deepseek-ai/deepseek-harness
Commit: `5dda764ed3aa172535a7967b06ff95d9cbfe536a` (see `FETCH_COMMIT.txt`)
Cached: `reference_architecture/deepseek-harness/` (~16 MB, 1600+ files;
`.git`, `node_modules`, build output, `.zh.md` translations and `.yaml`
i18n sidecars excluded).

Central idea: **everything is a plugin**. The product is a Cordis plugin
tree composed at boot from ordered layers (profile → bundles → user
patches → CLI overlays). There is no privileged core: the model adapter,
tool registry, session log, system-prompt assembler and the agent loop
itself are all plugins owning stable `ctx.<key>` services.

Conventions below: `Path` is relative to this cache dir. `AETHER
equivalent` names the current AETHER crate/file that owns the same
concern (or `none`). `Port` is the strategy actually adopted in
`IMPLEMENTATION_PLAN.md` / Phase 5 (`aether-runtime`).

---

## 1. Plugin framework (Cordis)

### docs/cordis-primer.md
- Subsystem: Plugin framework doctrine.
- Responsibility: Defines Context-as-service-repository, plugin shapes,
  `inject` ordering, dispatch modes (`emit`/`waterfall`/`parallel`/
  `serial`/`bail`), waterfall `next()` delegation semantics,
  reversible effects (`ctx.effect`/`ctx.on`, disposer per registration).
- Dependencies: Cordis (`@deepseek-ai/cordis`, vendored conceptually).
- Lifecycle: Read-first; every other row assumes it.
- AETHER equivalent: none (closest: `aether-plugin` middleware hooks,
  which have no lifecycle, no DI, no disposal).
- Port: Reimplement mechanics natively in `aether-runtime`: service
  registry, `inject`-gated activation, typed mode-tagged bus, effect
  stack with reverse-order disposal.

### docs/cordis-api/ + docs/cordis-tutorial/ + docs/cookbook/
- Subsystem: Plugin framework API/tutorial.
- Responsibility: Authoritative `Context`/`Service`/`Schema` API,
  plugin authoring tutorial, recipes.
- AETHER equivalent: none.
- Port: Not ported as code; distilled into
  `crates/aether-runtime/docs/PLUGIN_AUTHORING.md` (Phase 5).

### packages/core/scope/ (`src/index.ts`, `src/store.ts`, README)
- Subsystem: Scoped-registration primitive.
- Responsibility: `createScope(ctx,key)` (opaque `ScopeKey`, usually the
  live `Agent`); `ScopedLayers`/`NamedEntries`/`AnonymousEntries`
  (one eager global layer + lazy per-scope layers, most-specific-wins
  shadowing); `scopeTarget` carrier + ancestor/descendant dispatch
  filter; `ToolRestriction{allow,deny}` intersection with own-scope
  exemption; parent-chain binding (`composeFrom` exact generation).
- Dependencies: Cordis only (deliberately below session/loop).
- Lifecycle: Scope lives with its agent; `dispose()` unwinds all
  scope-owned registrations.
- AETHER equivalent: none (`canonical_toolsets()` are process-global
  constants; no per-session visibility).
- Port: `aether-runtime::scope` — `ScopeKey`, global + per-scope tool
  layers with shadowing, `restrict` intersection, scope-filtered
  dispatch predicate. Consumed by the executor visibility check.

---

## 2. Boot / composition

### packages/boot/app-boot/ (`src/index.ts`, `src/profile.ts`, README)
- Subsystem: Application boot + profile composition.
- Responsibility: `boot()` (host-prep vs tree-load error labels,
  `assertEntriesActivated` pending-service audit, fail-loud install);
  `PROFILE_TEMPLATES` (web/headless/sdk/sdk-minimal/acp);
  `composeEntries` (flatten layers → id-targeted whole-config
  replace, unknown target = warning); `renderConfigDump`
  (`--dump-config` with provenance comments); live vs startup
  `patchReload`; `!!js`-expression interpolation of `config`/`disabled`.
- Dependencies: Cordis loader, bundle manifests (`dsh.bundle.patch`).
- Lifecycle: Runs once per process (or watches patches when live).
- AETHER equivalent: `crates/aether-cli/src/run_task.rs:100-290`
  (hardcoded construction sequence), `crates/aether-desktop/src/main.rs`
  `run_task` (duplicated construction in a thread).
- Port: `aether-runtime::compose` — `Row{id,name,inject,disabled,
  only_os,requires_env,config}`, `compose()` with whole-config
  replace + provenance, `dump()` preview, `pending-service` audit on
  activation. Layer order: bundled base → `~/.aether/plugins.toml` →
  `--plugin-patch` overlays. AETHER-native: TOML rows, no `!!js`
  (explicit `only_os`/`requires_env` fields instead).

### packages/bundle/base/cordis.patch.yml (+ web-app/headless/sdk-app/sdk-minimal/acp-app/)
- Subsystem: Distribution bundles (first layer + mode layers).
- Responsibility: `dsh-base` mounts model adapters, tools,
  persistence, sandbox+approval, settings, credentials, telemetry;
  mode bundles restate rows fully (no merge) and disable base rows
  that move behind presets; `sdk-minimal` owns a complete standalone
  tree (proves replaceability).
- Dependencies: Every plugin package.
- Lifecycle: Static data applied at boot.
- AETHER equivalent: `run_task.rs:255-287` tool assembly block.
- Port: `base_profile()` in `aether-tools` (built-in tool-plugin
  rows: `tools-fs`, `tools-git`, `tools-terminal`, `tools-analysis`,
  `tools-memory`, `tools-skills`) + per-mode row restatement where
  needed. No deep merge anywhere.

### packages/preset/agent-presets/ (`presets/standard/agent.cordis.yml`, README)
- Subsystem: Agent presets (per-session composition).
- Responsibility: `cordis:group` + `isolate` realms (per-preset
  private service instances; root-publish rejected); standing mounts;
  `ctx.agentPresets.mount/composeFrom/recompose/serviceFor`;
  tool-only rows register into host `ctx.tools` scoped layer.
- Dependencies: `dsh-scope`, registries that stay host-visible
  (`jobs`, `skill`, `subagents`, `tokenMeter`, …).
- Lifecycle: Standing mount per preset; agent scope parents to it;
  `recompose` only while blank.
- AETHER equivalent: none (one global tool map per process,
  `Executor.allowed_tools` static allowlist).
- Port (Phase 5, partial): session-scoped `restrict` overlays on the
  tool service + `serviceFor`-style lookup; full preset standing
  mounts deferred (see IMPLEMENTATION_PLAN.md P3).

---

## 3. Agent runtime

### packages/core/agent/ (`src/types.ts`, `src/runtime-types.ts`, `src/index.ts`)
- Subsystem: Agent interface + live registry (`ctx.agents`).
- Responsibility: Minimal durable identity (`Agent{id == session.id}`)
  + live face (`session`, `inbox`, `status`, `ctx`, `send/followup/
  steer/inject/cancel/whenIdle/runMaintenance`); `AgentRegistry`
  (`create/resume/register/enter/announce`, initiator tracking);
  `agent/*` event vocabulary (created/disposed/status/inbox/*/
  session-start/pre-step/request/request-error/assistant-stream/
  turn-stopping/error).
- Dependencies: `dsh-scope` (carrier), `core/session`.
- Lifecycle: create → setup (unpublished) → enter → session-start →
  drive → stop-and-drain → unwind scope → detach.
- AETHER equivalent: `crates/aether-core/src/agent_loop.rs`
  (`Agent::run`), `task_state.rs` (`TaskStateMachine`, `TaskEventKind`).
- Port: Keep AETHER's state machine (stronger: typed transitions,
  Reviewer-only completion, doom-loop). Adopt: inbox vocabulary
  (`followup`/`steer`/`inject` → next-turn/next-step + wake latch) as
  a durable `inbox/spliced`-style projection in a later phase; adopt
  `agent/pre-step` + `agent/request` waterfalls now as
  `agent/pre-step`-equivalent prompt-assembly hook and
  `tools/*` execution hooks (Phase 5 wires the tool pair).

### packages/core/agent-loop/ (`src/agent.ts`, `src/tool-calls.ts`, README)
- Subsystem: Default loop driver (`ctx.agentLoop`).
- Responsibility: `Phase{idle,maintenance,running}` + `wakeRequested`
  latch; exact turn order (turn/start → claim → assemble →
  pre-step waterfall → step/start → request waterfall → prepareCall →
  sync admission commit → stream → tool calls → step/end →
  turn-stopping (serial) → turn/end); staged tool scheduler
  (ordered prepare, concurrent dispatch pool, model-order commit);
  cooperative `AbortController` cancellation; retry without
  re-assembly; `request/header` series logic.
- Dependencies: `core/agent`, `core/session`, `core/tools`,
  `core/system-prompt`, `llm/llm`.
- Lifecycle: Drives one agent; idle between turns.
- AETHER equivalent: `agent_loop.rs` (plan→execute→verify→replan
  budget loop) + `executor.rs` (model→tool-call loop).
- Port: Adopt staged-commit discipline (async proposal → sync
  commit; cancellation during async commits nothing) as a review
  rule for executor edits; adopt ordered-prepare/concurrent-dispatch/
  ordered-commit scheduler shape for parallel tool calls (P3).
  Do NOT replace the plan→verify loop — it is AETHER's domain logic.

### packages/core/session/ (`src/types.ts`, `src/surface.ts`, README)
- Subsystem: Session log + store (`ctx.sessions`).
- Responsibility: Append-only `SessionEvent{seq,time,type,data}`;
  `SessionEventMap` (turn/step/system/user/assistant+attempt/
  tool-call+result/request-header+context/inbox-spliced + merged
  plugin events); `deriveMessages()` pure fold (model-visible ⟺
  logged invariant); `surfaceOp` append/replace rules;
  `SESSION_FORMAT_VERSION` + adjacent migrations; `ctx.sessionPersistence`
  seam (single-writer, flush barrier, crash-prefix repair);
  `ctx.sessionProjections` (pure `init/apply`, `stateOf/snapshot/
  checkpoint/restore`).
- Dependencies: persistence provider (`session-persistence-jsonl`).
- Lifecycle: Append → fold → project; generations immutable.
- AETHER equivalent: `crates/aether-sessions` (sqlite `sessions.db`,
  `messages` + typed `message_parts`, `truncate_after`, snapshots).
- Port: Keep sqlite store. Adopt: "model-visible ⟺ logged" as an
  invariant test on the executor request path (P2); adopt pure
  projection-fold shape for future inbox/turn-boundary state (P3).
  Do NOT migrate storage format.

### packages/core/system-prompt/ (`src/index.ts`, README)
- Subsystem: Prompt assembly (`ctx.systemPrompt`).
- Responsibility: `section/context/tools/variable` registrations with
  numeric orders; global+scope merge with shadowing; `assemble()`
  + scope-filtered `system-prompt/assemble` waterfall (authoritative);
  strict `{{var}}` interpolation; surface-node reconciliation
  (in-history capable vs consolidate-at-head).
- Dependencies: `dsh-scope`.
- Lifecycle: Re-assembled per step; `system-prompt/change` (unfiltered)
  notifies all scopes.
- AETHER equivalent: `crates/aether-core/src/prompt.rs`
  (`PromptBuilder`, largely unused by hot path) + `CODER_SYSTEM` /
  `KARPATHY_POLICY` consts + `aether-skills` `PromptCompiler`.
- Port: Route skill/prompt contributions through section-style
  registrations with disposers where cheap (P2); keep the compiled
  kernel (token efficiency is an AETHER strength).

---

## 4. Tools

### packages/core/tools/ (`src/index.ts`, `src/schema.ts`, README)
- Subsystem: Tool registry + guarded pipeline (`ctx.tools`).
- Responsibility: `ToolDefinition` (parameters + `execute` +
  `finalizeContent` + `isConcurrencySafe` + presentation views);
  `register/restrict/guard/get/schemas/execute`;
  `ScopedLayers<ToolLayer>` (global + per-scope shadowing,
  `run_code` reservation); `mode: native|ptc|both` wire shaping;
  staged scheduler (`prepare/dispatch/finalize/finish`).
- Dependencies: `dsh-scope`, `ctx.approval` (serviceAsk).
- Lifecycle: Register (effect) → restrict → execute → dispose.
- AETHER equivalent: `crates/aether-tools/src/lib.rs` (`Tool` trait),
  `registry.rs` (`ToolRegistry`, `canonical_toolsets()`),
  `executor.rs:343-429` (allowlist + permission + body).
- Port (Phase 5, core seam): `ToolService` on the host —
  `register` (disposer), scoped `restrict{allow,deny}` with
  intersection + own-scope exemption, `resolve(name,scope)`
  (filtered-away ⇒ `UNKNOWN_TOOL`), `tools/pre-execute` waterfall
  (`allow|deny|ask`), monotonic `guard()` (deny-only), `tools/execute`
  around-dispatch, `tools/post-execute` (`accept|block`), `tools/result`
  emit. Executor consults pre/guard/post around its existing
  permission logic (which is preserved, not replaced).

### docs/tool-execution-pipeline.md + docs/tool-catalog.md + docs/subsystems/tools.md
- Subsystem: Tool pipeline contract + catalog.
- Responsibility: Exact phase order, no-arg-rewrite rule (logged args
  must equal executed args), lossless materialize + deep-freeze,
  error taxonomy (`UNKNOWN_TOOL`, `INVALID_ARGS`, `ABORTED*`,
  `TOOL_TIMEOUT`), timeout-policy as wrapper plugin.
- AETHER equivalent: `executor.rs` body + `ToolError` enum.
- Port: Enforce no-rewrite + error taxonomy mapping in the seam
  adapter; timeout wrapper as a post-hoc plugin listener (P2).

---

## 5. Model seam

### packages/llm/llm/ (`src/index.ts`, `src/types.ts`, `src/call-config.ts`)
- Subsystem: Model adapter seam (`ctx.llm`).
- Responsibility: `LlmAdapter` (wire ownership) + `LlmRuntime`
  (`registerAdapter` atomic + `replace`, `prepareCall` capability
  binding, `stream`); message/stream vocabulary (`ContentBlockMap`,
  `StreamChunk`, `FinishReasonMap`); `llm/stream` waterfall;
  adapter-generation binding (HMR cannot mix capability + endpoint).
- Dependencies: provider plugins (`llm-deepseek`, `llm-pi-ai`,
  `llm-replay` for tests).
- Lifecycle: Register → resolve → prepare (freeze) → stream (one-shot).
- AETHER equivalent: `crates/aether-gateway` (`ModelGateway`,
  `provider_for`, `validate_binding`, fingerprints) +
  `crates/aether-models` (`ModelProvider`, `OpenAICompatibleProvider`).
- Port: Keep the gateway (explicit binding + validation is an
  AETHER strength). Adopt: register the gateway handle as a host
  service with `replace()` so a plugin row can swap providers
  without editing `run_task.rs` (Phase 5 seam registration);
  capability precheck before prompt admission (P2).

---

## 6. Cross-cutting seams (all three-role: Definition/Provider/Consumer)

| Path | Seam (`ctx` key) | AETHER equivalent | Port |
|---|---|---|---|
| `packages/fs/fs` + `fs-local`/`fs-sandbox` + `tool-fs` | `ctx.fs` (resolve/stat/read/write/edit + versions; fenced by shared sandbox policy) | `aether-tools` `LocalWorkspace` + `sandbox_check` | Keep impl; expose as host service with local provider; sandbox-policy plugin point (P3) |
| `packages/shell/*` + `bash-local`/`bash-sandbox` + `tool-bash` | `ctx.shell` (`resolve(request)→spec` then `run/start`; request/spec split) | `ExecuteCommandTool` (`sh -c`) | Adopt request/spec split in a `ShellService` wrapper (P3); keep policy routing |
| `packages/sandbox/*` | `ctx.sandbox` (argv wrapping before spawn) | `sandbox_check` path confinement | Keep; wrap as service later (P3) |
| `packages/subagent/*` | subagent providers (child agent → delegated turn) | `subagents.rs` + `AgentDefinition`/`AgentRouter` | Keep; provider-swap seam later (P3) |
| `packages/settings/*` | `ctx.settings` (Schemastery sections, `setSource/onChange`, no rebuild) | `aether-config` TOML load-once | Keep file config; add `onChange`-style reload hook point (P3) |
| `packages/storage/*` | storage providers | `SessionStore`, `Mind` (redb), `WorkspaceStore` | Keep; no seam churn (P3 optional) |
| `packages/mcp/*` | MCP client + tool bridging | `aether-tools/src/mcp.rs` (`McpClient`, `McpTool`) | Mount each server as a plugin row contributing tools as effects (Phase 5) |
| `packages/skill/*` | `ctx.skill` registry | `aether-mind` `SkillRegistry` + `aether-skills` compiler | Register skill index as host service; skill tools contributed via plugin rows (Phase 5) |
| `packages/jobs/*` | `ctx.jobs` (background work, `job_*` tools) | desktop `background.rs` + CLI `--background` detached child | Keep; unify behind jobs-style service later (P3) |
| `packages/compaction/*` + `docs/subsystems/compaction.md` | compaction-basic (fold log → summary events) | `aether-context` `SessionCompactor` + checkpoint store | Keep LLM-checkpoint design; adopt prune/summary event vocabulary in task events (P2) |
| `packages/hooks/hook-protocol/` | hook invocations as logged events (`hook/invoked|result`) | `aether-plugin` 12-hook middleware | Bridge: publish plugin-hook transitions on the runtime bus as observe-events (P2) |
| `packages/session/session-projection/` | `ctx.sessionProjections` | none | Generic fold-registry pattern reused for runtime state snapshots (P3) |
| `packages/terminal/*`, `packages/shell/*` | `ctx.terminals`, `ctx.shell` backends | none in core (desktop owns PTY) | Out of scope |
| `packages/plan|goal|todo|workflow|schedule|spill/` | task-board primitives over subagents | `eng::LoopEngine`, `TaskStateMachine`, `EvidenceBag` | Keep; borrow `todo/write`-style durable-event vocabulary only (P3) |

---

## 7. Docs / governance

| Path | Responsibility | Port |
|---|---|---|
| `docs/architecture.md` | Composition doctrine (profiles/bundles/patches, turn flow, seams table) | This manifest + plan mirror its structure for AETHER |
| `docs/event-producer-consumer.md` | Every event's mode/producers/consumers (generated) | `aether-runtime` event catalog table in code docs + freshness test |
| `docs/capability-seams.md` | Seam graph (key/role/owner/impls/consumers, generated) | Seam table maintained in `aether-runtime` docs |
| `docs/persistence-catalog.md` + `docs/subsystems/persistence.md` | Event durability contract, generations, crash repair | Durability rules referenced by gap analysis; no format change |
| `docs/config-catalog.md` | Generated config-field reference | `--dump-plugins` output + commented `plugins.example.toml` |
| `docs/testing.md` | Real-registry-behind-mock-adapter harnesses, contract specs, recorded sessions | Copied as test strategy for `aether-runtime` (mock plugins, ordering/race specs) |
| `docs/defensive-patterns.md` | Fail-loud activation, root-publish rejection, strand detection | Adopted as host boot audit rules |
| `AGENTS.md` / `SAFETY.md` | Agent contributor rules / safety notice | AETHER keeps its own; review parity noted in gap analysis |
