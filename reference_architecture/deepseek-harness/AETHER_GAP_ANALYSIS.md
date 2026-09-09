# AETHER Gap Analysis — DeepSeek Harness vs Current AETHER (v0.28.0)

Reference commit `5dda764`. AETHER paths relative to repo root.
“CURRENT AETHER” is factual (surveyed `crates/*/src`, `run_task.rs`,
`main.rs`, `executor.rs`, `agent_loop.rs`).

---

## 1. Plugin Architecture

- CURRENT AETHER: `crates/aether-plugin` is in-process typed
  middleware: `Plugin{info, register, 12 default no-op hooks}`,
  `Registry` with 12 `HookChain`s, `HookOutcome::Continue/Halt`,
  global `OnceLock` bus. No dynamic loading, no lifecycle, no DI, no
  disposal, no config-file mounting. Composed never — `Agent` just
  calls `plugins.on_session_start/end`, `on_agent_spawn/complete`.
- DEEPSEEK HARNESS: Cordis `Context` service repository; function or
  `Service`-class plugins with `inject` ordering, `Config` validation,
  `apply`/`dispose`; registrations are reversible effects; boot
  composes profile → bundles → patches with whole-config replace and
  `--dump-config`; activation audited (`pending (waiting for …)`).
- GAP: AETHER has middleware callbacks, not plugins. Nothing can be
  added, removed, replaced or reconfigured without editing Rust code.
- ROOT PROBLEM: No runtime object owns composition. `run_task.rs`
  *is* the composition, written as imperative construction.
- TARGET: `aether-runtime::PluginHost` — plugin trait with
  id/version/inject/apply/dispose, service registry, effect stack,
  TOML-row composition with provenance + dump, pending-service audit.
- AETHER IMPLEMENTATION: New crate `aether-runtime` (Phase 5);
  built-in tool plugins in `aether-tools/src/plugins.rs`; host built
  in `run_task.rs`; `--dump-plugins`.

## 2. Agent Runtime

- CURRENT AETHER: `Agent::run` (~1100 lines, `agent_loop.rs`):
  plan→execute→verify→replan budget loop over a `TaskStateMachine`
  (16 typed states, validated transitions, Reviewer-only completion,
  doom-loop detection) + `Executor` model→tool-call loop. `Mode::Build/
  Plan` only changes prompt text. No loop-driver seam; no inbox;
  steering = new run. Cancellation cooperative at iteration boundary.
- DEEPSEEK HARNESS: `Agent` interface + live registry + `agent/*`
  vocabulary; driver is a plugin (`ctx.agentLoop`); turn/step machine
  with durable-vs-live split, inbox (`followup`/`steer`/`inject` →
  next-turn/next-step + wake latch), `pre-step`/`request` waterfalls,
  `prepareCall` capability binding, sync admission commit,
  serial `turn-stopping`, per-turn `AbortController`.
- GAP: AETHER's loop is a monolith with no interception points and no
  durable input queue; prompt assembly and request binding are inline.
- ROOT PROBLEM: Domain logic (plan→verify) and runtime mechanics
  (admission, cancellation, interception) are fused in one function.
- TARGET: Keep the plan→verify loop and state machine (AETHER
  strengths). Extract runtime mechanics: tool-execution interception
  (Phase 5), prompt-assembly hook + capability precheck (P2), durable
  inbox projection (P3).
- AETHER IMPLEMENTATION: Executor consults host `tools/pre-execute`
  waterfall + guards + `tools/post-execute`; `run_task.rs` builds the
  tool map through plugin rows. No `Agent::run` restructuring in
  Phase 5.

## 3. Tool Architecture

- CURRENT AETHER: `Tool` trait
  (`name/description/json_schema/category/required_permission/
  execute`) + `ToolRegistry` (BTreeMap, replace-by-name) +
  `canonical_toolsets()` planner/executor/reviewer constants +
  `Executor.allowed_tools` static allowlist. Permissions resolved
  inline (`executor.rs:371-426`). No interception, no per-session
  visibility, no arg-immutability rule, no error taxonomy.
- DEEPSEEK HARNESS: `ToolDefinition` + scoped registry
  (`ScopedLayers`, shadowing, `restrict{allow,deny}` intersection,
  own-scope exemption, `run_code` reservation); pipeline
  pre→ask→guard→execute→post→finalize→result; args immutable after
  logging; error taxonomy; staged scheduler (ordered prepare,
  concurrent dispatch, ordered commit).
- GAP: Static, process-global, uninterposable tool surface.
- ROOT PROBLEM: Registration is construction code, not data; policy
  lives inside the executor body.
- TARGET: `ToolService` on the host: `register` (disposer), scoped
  `restrict`, `resolve` (filtered-away ⇒ `UNKNOWN_TOOL`), pre/guard/
  post/result interception, error-taxonomy mapping.
- AETHER IMPLEMENTATION: Phase 5 — full seam + executor integration
  (pre-w waterfall allow/deny/ask mapped onto existing permission
  flow; post-w accept/block; scoped check before allowlist).
  Existing permission engine is preserved and runs inside the
  pipeline, not replaced.

## 4. Context Integration

- CURRENT AETHER: Three systems — `ContextManager` per-agent
  segments + `CompactionEngine`; `SessionCompactor` (Model 2 writes
  `CompactionCheckpoint` → atomic persist → rebuild); `ContextWorkspace`
  (per-agent managers + shared pinned layer). Plus `PromptBuilder`
  (mostly unused) and `aether-skills` `PromptCompiler` (used).
- DEEPSEEK HARNESS: `ctx.systemPrompt` assembly from
  section/context/tools/variable registrations + authoritative
  `assemble` waterfall; runtime context projected as `user/message`
  (durable, therefore logged); surface-node reconciliation;
  compaction as log-folding plugins emitting summary/prune events.
- GAP: AETHER's prompt contributions are hardcoded consts; skill
  injection has one hook (`build_coder_system`); injected context is
  not uniformly logged as model-visible state.
- ROOT PROBLEM: No contribution registry for prompt assembly.
- TARGET: Section-style prompt contributions with disposers;
  runtime-context-as-logged-message discipline; keep the compiled
  kernel + LLM-checkpoint compaction.
- AETHER IMPLEMENTATION: P2 (section registry behind the skill
  compiler; `agent.inject()`-equivalent logged injection). Phase 5
  lays the event/registry machinery only.

## 5. Event Architecture

- CURRENT AETHER: `RuntimeEventBus` (per-category broadcast,
  256-cap, off the hot path) + `TaskEventKind` (task lifecycle,
  parallel infra) + Tauri `task-output/task-state/task-exit` bridge.
  No dispatch modes, no waterfall, no scope filtering, no catalog.
- DEEPSEEK HARNESS: One bus, five modes
  (`emit/waterfall/parallel/serial/bail`) declared per event via
  `@mode`; scope-filtered dispatch (`scopeTarget` carrier);
  generated producer/consumer matrix + declaration-vs-dispatch CI
  check; `SessionEventMap`/`ContentBlockMap` merge-extensible
  vocabularies.
- GAP: No interception semantics; observers only.
- ROOT PROBLEM: Events were added for observability, never as
  extension points.
- TARGET: Mode-tagged bus in `aether-runtime` (`Emit/Waterfall/
  Serial/Parallel`), scope predicate, event catalog table + freshness
  test. `tools/*` pair is the first real consumer.
- AETHER IMPLEMENTATION: Phase 5 bus + catalog test; bridge existing
  `aether-plugin` hook transitions as observe-events (P2).

## 6. State Management

- CURRENT AETHER: `TaskStateMachine` (excellent: validated matrix,
  history, doom tracking, repair/replan caps) + sqlite session rows +
  `message_parts` + snapshots + kv. State transitions are data-driven
  but state *ownership* is scattered (`Agent`, `Executor`,
  `LoopEngine`, stores).
- DEEPSEEK HARNESS: Append-only `SessionEvent` log as the single
  source of truth; `deriveMessages()` pure fold; model-visible ⟺
  logged invariant (compile + test enforced); `sessionProjections`
  (pure `init/apply`, `stateOf/snapshot/checkpoint/restore`).
- GAP: AETHER's model-visible history (executor `messages: Vec`) is
  ephemeral and reconstructable only via session re-hydration
  conventions, not by construction.
- ROOT PROBLEM: No single artifact derives both the wire request and
  the persisted transcript.
- TARGET: Invariant test — every executor request field must be
  reconstructable from the session log prefix (P2); pure
  fold-registry for inbox/turn-boundary state (P3). No storage
  migration.
- AETHER IMPLEMENTATION: P2 invariant test + projection-registry
  utility in `aether-runtime` (generic, reused for host state
  snapshots).

## 7. Configuration

- CURRENT AETHER: `config.toml` (agent/memory/permissions/context/
  models/display/subagents/mcp/frontend/appearance) + canonical
  `providers.json` + per-session `RoleAssignments`. Loaded once at
  startup; no reload, no layering, no composition rows, no dump.
- DEEPSEEK HARNESS: Layered rows (bundles → profile patch → home
  patch → `--patch`), whole-config replace, `disabled`/`config`
  expressions, live reload for interactive profiles, `--dump-config`
  with provenance.
- GAP: No layering, no patching, no introspection, no reload.
- ROOT PROBLEM: Config is a struct, not a composition.
- TARGET: `plugins.toml` layering for the plugin tree (base →
  `~/.aether/plugins.toml` → `--plugin-patch`), whole-row replace,
  `disabled`/`only_os`/`requires_env`, `--dump-plugins`. App config
  (`config.toml`) untouched.
- AETHER IMPLEMENTATION: Phase 5 compose module + CLI flags
  (`--dump-plugins`, `--plugin-patch …`) + `plugins.example.toml`.

## 8. Extensibility

- CURRENT AETHER: Add a tool = new `Tool` struct + edit
  `run_task.rs` + registry. Swap provider = edit `build_provider`.
  Custom loop = fork `agent_loop.rs`. Swap prompt = edit consts.
  Second memory backend = rejected by single-slot guard + edits.
- DEEPSEEK HARNESS: Add capability = register on `ctx.tools`;
  swap provider = register adapter on `ctx.llm`; per-session set =
  preset with `isolate`; intercept = listen to `agent/*|tools/*`;
  durable input = extend `SessionEventMap`. The "where new behavior
  goes" table is documentation, and it holds.
- GAP: Every extension requires a code change in core files.
- ROOT PROBLEM: No mounting surface exists.
- TARGET: An AETHER "where new behavior goes" table that holds:
  tool → plugin row; tool policy → guard listener; prompt section →
  section registration; provider swap → gateway service `replace()`;
  per-session tools → scoped `restrict`; durable input → session event.
- AETHER IMPLEMENTATION: Phase 5 delivers rows 1–2 + 5 of that table;
  P2–P3 complete it.

## 9. Persistence

- CURRENT AETHER: sqlite (`sessions.db`: sessions/messages/parts/
  tool_calls/checkpoints/kv/traces) + per-session snapshot chains +
  redb mind + `workspaces.db`. Solid, transactional, session-scoped.
  Crash recovery via `truncate_after` + `recovery_state`.
- DEEPSEEK HARNESS: JSONL generations (`session.vN.jsonl[.zstd]`),
  adjacent static migrations, single-writer + flush barrier,
  crash-prefix repair, `interruptedTurnClosers` on resume.
- GAP: None material — different trade, not a deficiency. AETHER's
  sqlite gives atomicity the JSONL design works to recover.
- ROOT PROBLEM: n/a.
- TARGET: Keep sqlite. Borrow vocabulary (`interrupted`-style
  closers already exist via `recovery_state`) and the
  write-barrier discipline for background detach (P3, optional).
- AETHER IMPLEMENTATION: No persistence changes in Phase 5.

## 10. Error Handling

- CURRENT AETHER: `ToolError::Io/Anyhow/Other`,
  `ProviderError::Http/Json/Api/ApiStatus/MissingEnv`,
  `GatewayError::NotConfigured/Cancelled/CapabilityDenied/Provider`,
  `FailureClass` HTTP mapping, `classify_probe`, bounded error
  strings, redaction at boundaries. Good taxonomy at the gateway;
  flat (`Other(format!)`) inside tools.
- DEEPSEEK HARNESS: `HarnessError{name,code}` preserved across the
  pipeline, flattened otherwise; per-phase error events
  (`request-error` → retry ownership without `next()`);
  `UNKNOWN_TOOL/INVALID_ARGS/INVALID_TOOL_OUTPUT/ABORTED*/TOOL_TIMEOUT`;
  fail-loud activation audit; strand detection.
- GAP: Tool-layer errors are untyped strings; no retry-ownership
  protocol; boot failures are silent-ish (stderr + exit code).
- ROOT PROBLEM: Errors were typed where providers meet the gateway,
  never where tools meet the loop.
- TARGET: Map tool outcomes onto a taxonomy at the seam boundary
  (`UnknownTool/InvalidArgs/Denied/Aborted/Timeout/Failed`) while
  keeping `ToolError` wire-compatible; fail-loud host boot audit.
- AETHER IMPLEMENTATION: Phase 5 taxonomy mapping + boot audit;
  `ToolError` itself unchanged (P3 may type it).

## 11. Testing

- CURRENT AETHER: 211+ unit tests per crate (state machine 22,
  gateway 45 incl. connection-refused-no-hang, sessions 10, tools
  18…), mock provider/workspace harnesses
  (`aether-core/src/testing.rs`, `tests/agent_loop_mock.rs`).
- DEEPSEEK HARNESS: Real-registry-behind-mock-adapter harnesses,
  contract-regression specs (ordering/races), recorded-session
  snapshots for transcript-visible changes, per-file coverage as
  dead-code detector, catalog freshness checks.
- GAP: Ordering/race contracts and catalog freshness are untested;
  harness runs mocks, not the real registry+pipeline.
- ROOT PROBLEM: Tests assert units, not composition.
- TARGET: `aether-runtime` ships ordering/disposal/waterfall/race
  contract tests + catalog freshness test; executor seam test runs
  the real pipeline (real registry + mock tool + mock host
  listeners).
- AETHER IMPLEMENTATION: Phase 5 test suite (~40 tests); P2 adds the
  model-visible⟺logged invariant test.

## 12. Lifecycle

- CURRENT AETHER: Constructors (`new`/`with_*` chains, 16-arg
  `Agent::new`), cooperative `Notify` cancellation, no disposal
  protocol (Arc drops), no activation order (call order = order).
- DEEPSEEK HARNESS: `inject`-gated activation (order-independent),
  two-stage boot errors, scope-bound lifetimes, reverse-order
  disposal, `rawDispose` for ordered composites, HMR-safe
  re-registration, generation turnover for presets.
- GAP: Implicit lifecycles; teardown is "hope Arc drops";
  reconfiguration requires restart; no pending-dependency diagnosis.
- ROOT PROBLEM: Rust ownership was used as a substitute for a
  lifecycle design.
- TARGET: Explicit host boot (`boot()` → activation audit) and
  `shutdown()` (reverse disposal); `inject` deps replace call-order
  dependence for tools/services; disposers for every registration.
- AETHER IMPLEMENTATION: Phase 5 host boot/shutdown + effect stack;
  agent loop keeps RAII (no rewrite), plugins get real lifetimes.

## 13. Dependency Management

- CURRENT AETHER: Cargo workspace, 18 crates, path deps, clear
  layering (models ← gateway ← core ← cli/desktop). Duplicated
  config structs in desktop for build independence (documented
  reason). `Cargo.lock` committed.
- DEEPSEEK HARNESS: pnpm workspace, 50+ packages, `dsh` manifest
  fields (`dsh.profile`/`dsh.bundle`), catalog-pinned deps,
  entrypoint verifier (`verify-application-entrypoints`), no
  direct in-process mounting except tests.
- GAP: None structural. AETHER's layering is sound; the missing
  piece is *runtime* dependency resolution (`inject`), not build
  deps.
- ROOT PROBLEM: n/a.
- TARGET: Keep Cargo layering. `aether-runtime` sits at the bottom
  (deps: serde/serde_json/toml/thiserror/anyhow/tokio/async-trait
  only — all already in lockfile). `aether-tools` and `aether-core`
  depend on it; it depends on nothing AETHER.
- AETHER IMPLEMENTATION: Phase 5 `Cargo.toml` wiring; no new
  external crates (TOML rows, not YAML, for exactly this reason).

---

## Strengths to preserve (do NOT regress)

Explicit `provider+model` per role (no routing/fallback); live
`validate_binding` + `fingerprint_binding` + `ValidationStore`;
Reviewer-only completion + evidence requirement + doom-loop caps;
`sandbox_check`/`LocalWorkspace`/git-option-guard/snapshot
confinement/session-id allowlist/`redact_secrets`/extra_body+header
blocklists/`sanitize_image_url`/timeouts; atomic checkpoint
compaction with session isolation; `threat::sanitize` on untrusted
project context; `EvidenceBag.decide()` grounded verification;
compiled skill kernel (6256→713 tokens).
