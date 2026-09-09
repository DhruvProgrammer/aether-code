# Implementation Plan — Plugin-First AETHER (DeepSeek Harness port)

Follows `REFERENCE_MANIFEST.md` (what/where) and `AETHER_GAP_ANALYSIS.md`
(why). Prioritized by value ÷ blast-radius. P0 ships as the Phase 5
change set; P1–P3 are sequenced follow-ups with explicit entry criteria.

Design rule for the whole port: **adopt mechanics, keep domain logic**.
AETHER's plan→verify loop, state machine, gateway explicitness,
sandboxing, redaction, sqlite persistence and compiled skill kernel are
domain strengths and are NOT replaced. What is ported is composition,
interception, scoping, lifecycle and configuration machinery.

Conventions: `dsh:<path>` = cached reference source;
`aether:<path>` = repo path. New crate `aether-runtime` depends only on
crates already in `Cargo.lock` (serde, serde_json, toml 0.8,
thiserror, anyhow, tokio, async-trait) — no new external deps.

---

## P0 — Plugin runtime + tool seam + composition (THIS phase)

Value: every future extension mounts instead of forking. Blast radius:
additive (new crate; two opt-in integration points with `None` =
current behavior).

### P0.1 Subsystem: runtime core — plugin/service/effect
- Reference source: `dsh:docs/cordis-primer.md`,
  `dsh:packages/core/scope/README.md` (disposal),
  `dsh:docs/defensive-patterns.md`.
- AETHER files affected: `aether:Cargo.toml` (member),
  `aether:crates/aether-runtime/*` (new).
- New modules: `plugin.rs`, `services.rs`, `effects.rs`, `host.rs`.
- Data structures:
  - `PluginInfo{id, name, version, description}`
  - `trait Plugin: Send+Sync { info(); inject()->Vec<String> (default
    []); apply(&self, ctx:&mut PluginContext, config:JsonValue)
    -> impl Future<Output=Result<()>>; }` (async-trait; MCP connect
    needs await)
  - `PluginContext{ host, scope: ScopeKey, effects: EffectStack }`
  - `ServiceRegistry{ map: HashMap<&'static str, ServiceEntry{ value:
    Arc<dyn Any+Send+Sync>, owner: String, replaced: bool }> }`
  - `Effect{ label, undo: Box<dyn FnOnce()+Send> }`, `EffectStack`
    (single-shot, idempotent `run()`).
- Interfaces/traits: `Plugin`, `ServiceHandle<T>` (typed `get` with
  `MissingService{key, waiter}` error carrying the waiter id for the
  audit).
- Events: none yet (P0.2).
- Persistence: none.
- Tests: register/provide/get roundtrip; duplicate-service error
  names owner + contender; `inject`-gate activation order is
  order-independent (shuffle rows, assert same activation order);
  pending-service audit message lists waiter + missing keys;
  two-stage boot error labels (host-prep vs tree-load); shutdown
  disposes in reverse activation order (recorded probe).
- Migration concerns: none — new crate, no callers yet.

### P0.2 Subsystem: mode-tagged event bus + scope filter
- Reference source: `dsh:docs/cordis-primer.md#dispatch-modes`,
  `dsh:docs/event-producer-consumer.md`,
  `dsh:packages/core/scope/src/store.ts` (filter predicate).
- AETHER files affected: `aether:crates/aether-runtime/src/events.rs`,
  `scope.rs` (new).
- New modules: `events.rs`, `scope.rs`, `catalog.rs` (test).
- Data structures:
  - `enum Mode{ Emit, Waterfall, Serial, Parallel }`,
    `struct EventDef{ name:&'static str, mode: Mode }`
  - `ScopeKey(u64)` (global = 0; session scopes minted per run),
    `Scoped{ key: ScopeKey, ancestors: Vec<ScopeKey> }`
  - Listener: `Box<dyn Fn(payload)->BoxFuture<ListenerOutcome>+Send+Sync>`
    with `ListenerOutcome{ Continue(Value), ShortCircuit(Value),
    Deny(String), Block(Value) }` per-event-kind interpretation.
  - Waterfall contract: returning without delegating short-circuits;
    debug assertion + test that a non-delegating listener stops the chain.
- Interfaces/traits: `Bus::on(def, scope_filter, listener)->Effect`,
  `Bus::dispatch(def, scope, payload)->DispatchOutcome` (async).
- Events (first catalog — every entry gets `EventDef` + doc row):
  `tools/pre-execute` (Waterfall), `tools/post-execute` (Waterfall),
  `tools/result` (Emit), `tools/change` (Emit, unfiltered),
  `agent/pre-step` (Waterfall, reserved for P2),
  `host/booted`, `host/shutdown` (Emit).
- Persistence: none.
- Tests: waterfall delegation order + short-circuit stops chain;
  `must-call-next` positive/negative; serial awaits in order;
  parallel fans out and awaits all; emit is fire-and-forget and a
  panicking listener cannot break dispatch; scope filter admits
  untagged + self + descendants, rejects siblings; `tools/change` is
  unfiltered by construction; catalog freshness test (every `EventDef`
  has a doc row; every doc row has an `EventDef`).
- Migration concerns: bus is sync-constructible, `Send+Sync`, no
  global singleton (host-owned; tests get fresh hosts).

### P0.3 Subsystem: tool seam (Definition/Provider/Consumer)
- Reference source: `dsh:packages/core/tools/src/index.ts`
  (`register/restrict/guard/get/schemas/execute`,
  `ScopedLayers`, `ToolRestriction`),
  `dsh:docs/tool-execution-pipeline.md` (phase order, no-rewrite
  rule, taxonomy).
- AETHER files affected: `aether:crates/aether-tools/src/plugins.rs`
  (new — built-in tool plugins), `aether:crates/aether-core/src/
  executor.rs` (consult seam), `aether:crates/aether-cli/src/
  run_task.rs` (build via host).
- New modules: `aether-runtime::tools.rs` (`ToolService`); tool
  plugins in `aether-tools::plugins`
  (`FsPlugin, GitPlugin, TerminalPlugin, AnalysisPlugin,
  MemoryPlugin, SkillsPlugin, McpPlugin`).
- Data structures:
  - `ToolEntry{ name, owner: plugin id, tool: Arc<dyn Any…> }` —
    NOTE: to avoid a dependency cycle (`aether-tools → aether-runtime`
    for `Plugin`, `aether-runtime ↛ aether-tools`), the service
    stores contributor callbacks, not `Arc<dyn Tool>`:
    `ToolContributor{ name, owner, describe()->(String,String,Value),
    execute: Box<dyn Fn(Value, Cwd)->BoxFuture<Result<Value,ToolFault>>…> }`.
    Adapters in `aether-tools::plugins` wrap `Arc<dyn Tool>`.
  - `ToolRestriction{ allow: Option<HashSet<String>>, deny:
    HashSet<String> }`, per-scope overlay stack; `resolve(name,
    scope)` applies global insertion order + scope shadowing, then
    restriction intersection, then own-scope exemption.
  - `PreDecision{ Allow, Deny{reason}, Ask{reason} }`,
    `PostDecision{ Accept{content_override?}, Block{feedback} }`,
    `ToolGuard = Box<dyn Fn(&ToolExec)->Option<String>>` (deny-only),
    `ToolExec{ call_id, name, args: Value, scope }`,
    `ToolFault{ kind: UnknownTool/InvalidArgs/Denied/Aborted/Timeout/
    Failed, message }` mapped from `ToolError` at the boundary.
- Interfaces/traits: `ToolService{ register()->Effect,
  restrict(scope)->Effect, guard()->Effect, resolve(),
  pre_execute(), post_execute() }`.
- Events: `tools/pre-execute`, `tools/post-execute`, `tools/result`,
  `tools/change` (see P0.2).
- Persistence: none.
- Tests: shadowing (scope tool wins for its scope only); restrict
  intersection (two denies stack; allow∩allow narrows); own-scope
  exemption survives restriction; filtered-away ⇒ `UnknownTool`;
  guard cannot re-allow a pre-deny (monotonicity); post-block
  converts success to error with feedback; `tools/change` fires on
  register/dispose; no-rewrite: executed args equal logged args
  (test double records both).
- Migration concerns: `Executor` gains `with_plugin_host(Option<Arc<
  PluginHost>>)` + `with_tool_scope(ScopeKey)`; `None` = byte-identical
  current path (existing tests must pass untouched). `ToolError`
  unchanged.

### P0.4 Subsystem: composition (profile → patches → dump)
- Reference source: `dsh:packages/boot/app-boot/src/profile.ts`,
  `dsh:packages/bundle/base/cordis.patch.yml` (row shape, whole-config
  replace, `disabled` gating), `dsh:packages/boot/app-boot/src/
  index.ts` (`renderConfigDump`).
- AETHER files affected:
  `aether:crates/aether-runtime/src/compose.rs` (new),
  `aether:crates/aether-cli/src/run_task.rs` (layer order),
  `aether:crates/aether-cli/src/main.rs` (flags),
  `aether:plugins.example.toml` (new), `aether:~/.aether/plugins.toml`
  (user file, created lazily with commented example on first boot
  if missing — never overwritten).
- New modules: `compose.rs`.
- Data structures:
  - `Row{ id, plugin: String (registry name e.g.
    "builtin:tools-fs"), inject: Vec<String> (default derived from
    plugin), disabled: bool (default false), only_os:
    Option<String>, except_os: Option<String>, requires_env:
    Vec<String>, config: JsonValue (default {}) }`
  - `ComposedRow{ row, origin: Origin{Base, UserFile, Overlay(u16),
    Flag}, replaced: bool }`, `Composition{ rows: Vec<ComposedRow>,
    warnings: Vec<String> }`.
  - Layer order: `base_profile()` → user file → `--plugin-patch`
    files in order. Same id ⇒ whole-row replace (config NOT merged);
    unknown id in a patch ⇒ warning (not fatal); `disabled` /
    os-mismatch / missing env ⇒ skipped with reason recorded.
- Interfaces/traits: `PluginRegistry{ register_plugin(name, factory) }`
  (factories: `Fn()->Arc<dyn Plugin>`), `compose(layers)->Composition`,
  `dump(composition)->String` (TOML with `# origin:` comments).
- Events: `host/booted{ rows, skipped }`.
- Persistence: user file only; never writes back.
- Tests: whole-config replace (config keys do NOT merge); unknown-id
  warning; disabled/os/env skipping with reasons; provenance per row;
  dump round-trips through `compose`; empty user file = base only.
- Migration concerns: base profile rows default to exactly today's
  tool set (no behavior change on first boot); desktop does not read
  the file yet (P2) — CLI-only surface in P0.

### P0.5 Subsystem: host boot + CLI surface + docs
- Reference source: `dsh:packages/boot/app-boot/src/index.ts`
  (`boot()`, `assertEntriesActivated`),
  `dsh:docs/config-catalog.md` (generated reference),
  `dsh:docs/testing.md` (harness strategy).
- AETHER files affected: `run_task.rs` (host construction +
  `boot().await` before agent build; `shutdown()` after),
  `main.rs` (`--dump-plugins`, `--plugin-patch <file>…`),
  `crates/aether-runtime/docs/PLUGIN_AUTHORING.md` (new),
  `aether:DEPENDENCIES.md` (runtime row).
- Data structures: `BootReport{ activated: Vec<String>, skipped:
  Vec<(String,String)>, warnings: Vec<String> }` printed at
  `--debug` / on failure.
- Interfaces/traits: `PluginHost{ boot(composition), shutdown(),
  services(), bus(), tool_service(), mint_scope(), registry() }`.
- Events: `host/booted`, `host/shutdown`.
- Persistence: none.
- Tests: end-to-end boot test (base profile boots, all rows active,
  shutdown unwinds with empty effect stack); `--dump-plugins`
  golden test (row ids + origins stable).
- Migration concerns: boot failure = fail-loud with pending-service
  audit (never silent partial boot); `run_task` falls back to direct
  construction only if host boot is unreachable — NO, fail loud
  (matches reference; partial tool sets are worse than errors).

### P0 acceptance bar
`cargo check --workspace` clean (no new warnings);
`cargo test` all green incl. ~40 new runtime tests + executor seam
tests; `tsc`+`vite` untouched-green; `--dump-plugins` output stable;
default boot produces exactly today's tool set.

---

## P1 — Prompt contributions + gateway service + hook bridge (next)

Entry criteria: P0 merged, default boot behavior-identical for one release.
- Prompt sections: `PromptSection{ name, order, text|fn, scope }` registry
  in `aether-runtime`; skill compiler + `build_coder_system` publish
  sections instead of string concat; `system-prompt/change` (Emit,
  unfiltered); `agent/pre-step` waterfall reserved → consulted before
  `controller::plan` with `{enter,reject}` vocabulary.
- Gateway service: register the built `GatewayBundle` handle as host
  service `gateway` with `replace()`; a plugin row can swap providers
  without editing `run_task.rs`; capability precheck before prompt
  admission.
- Hook bridge: publish `aether-plugin` hook transitions on the runtime
  bus as observe-events (`plugin/hook` Emit with hook name + outcome).
- Model-visible⟺logged invariant test on the executor request path.
- Timeout-policy wrapper as a `tools/execute`-around listener.

## P2 — Durable inbox + projections + desktop composition (after P1)

Entry criteria: P1 sections carry 100% of today's prompt content.
- Durable inbox: `inbox/spliced`-style projection over session kv
  (`next_turn`/`next_step` + wake latch); `send/steer/inject`
  surface in CLI (`/steer`) and desktop; claim-on-turn-start.
- Projection registry: generic `init/apply/stateOf/snapshot/
  checkpoint/restore` in `aether-runtime`; inbox + turn-boundary as
  first units; missing key = absent capability.
- Desktop reads the same composition (base + user file); per-session
  `restrict` overlays from the role-settings UI.
- `plugins.toml` live-reload for CLI long-running mode
  (`patchReload: live` equivalent); one-shot commands stay
  start
...[truncated 1791 chars]