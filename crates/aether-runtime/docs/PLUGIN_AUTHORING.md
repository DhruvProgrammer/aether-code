# Plugin authoring guide (`aether-runtime`)

This is the AETHER-native distillation of the DeepSeek Harness
`cordis-primer` + `cordis-tutorial` material (see
`reference_architecture/deepseek-harness/docs/cordis-primer.md`).
Mechanics are ported; no Cordis code is used.

## Mental model

A running AETHER task is a **plugin tree composed at boot from
ordered layers**: bundled base rows → `~/.aether/plugins.toml` →
`--plugin-patch` overlays. Each row names a **factory** (code that
builds a [`Plugin`]) plus whole-row `config`. There is no
deep-merge: restating a row replaces it entirely.

## Writing a plugin

```rust
use aether_runtime::{Plugin, PluginContext, PluginInfo};
use async_trait::async_trait;

pub struct MyPlugin;

#[async_trait]
impl Plugin for MyPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo::new("my-tools", "My tools", "0.1.0", "what it does")
    }

    // Services that must exist first (the `inject` gate).
    // Missing services fail boot with a pending-service audit.
    fn inject(&self) -> Vec<String> {
        vec![]
    }

    async fn apply(&self, ctx: &mut PluginContext) -> anyhow::Result<()> {
        // Reject typos loudly-but-politely:
        ctx.warn_unknown_keys(&["timeout_ms"]);
        // ... register tools / services / listeners / guards ...
        // ... pushing every disposer onto ctx.effects ...
        Ok(())
    }
}
```

Register a factory so rows can name it:

```rust
host.register_factory("my-plugin", Box::new(|| Arc::new(MyPlugin)));
```

```toml
[[plugin]]
id = "my-tools"
plugin = "my-plugin"

[plugin.config]
timeout_ms = 60000
```

## Contributing tools

Wrap each tool with `aether_tools::plugins::adapt(tool, owner)` and
`ctx.host.tools.register(contributor)`, pushing the returned effect:

```rust
let effect = ctx.host.tools.register(adapt(tool, "my-tools"));
ctx.effects.push(effect);
```

Rules (enforced by tests, not just docs):

- **Disposal is exact.** The effect unregisters exactly your
  (scope, name, owner) triple. Shutdown unwinds in reverse.
- **Args are immutable.** Listeners see a snapshot; the pipeline
  executes the authoritative `ToolExec.args`. There is no
  rewrite path — logged args always equal executed args.
- **Hidden is missing.** A tool filtered away by `restrict` resolves
  to `None`, and the executor reports `unknown tool`, never
  `permission denied`. Do not leak capability existence.

## Intercepting execution

| Goal | Mechanism |
|---|---|
| Veto / escalate to approval | `tools/pre-execute` waterfall → `{decision: deny\|ask, reason}` |
| Deny unconditionally | `tools.guard(owner, f)` — deny-only, cannot re-allow |
| Rewrite / reject outputs | `tools/post-execute` waterfall → `{decision: accept, content_override?}` / `{decision: block, feedback}` |
| Observe settlements | `tools/result` emit (`{call_id, tool, ok}` — never full output) |
| React to membership | `tools/change` emit (unfiltered — every scope sees it) |

Waterfall contract: call `next.proceed(value)` to delegate (you may
wrap the downstream result on return); return without delegating to
short-circuit. Short-circuiting is a feature (policy owns the
decision), not a bug — but document it on the listener.

Guards are **monotonic**: `Fn(&ToolExec) -> Option<String>`, `None`
means no opinion. There is no allow path, so a guard can never undo a
waterfall denial, a policy denial, or another guard.

## Scoping tools to a session

```rust
// Only these tools are visible in this session (intersects with any
// other overlay on the chain):
tools.restrict(scope, ToolRestriction::allow(["read_file".into()]));
// Or hide some:
tools.restrict(scope, ToolRestriction::deny(["execute_command".into()]));
```

Overlays intersect across the chain; tools contributed *by* the
dispatch scope itself (non-global) are exempt. Resolve with
`tools.resolve(name, &ScopeChain::for_scope(scope))`.

## Services

Publish a capability with `services.write().await.provide(KEY, value,
owner)` (single-active — duplicates fail naming both parties) or
swap one with `replace()` (atomic; this is how a whole backend moves
without touching consumers). Consumers resolve per call with
`get::<T>(KEY, waiter)` — never import an implementation.

## Config

Rows carry whole-row `config` (`serde_json::Value`). Validate what
you read; `warn_unknown_keys` what you don't. Unknown keys warn in
the boot report — they never fail boot (forward compatibility for
downgraded binaries reading newer files).

## Boot discipline

- Fail loud: `apply` returns `Err` only when the row genuinely
  cannot function. The host unwinds already-activated rows first —
  no partial tool sets survive.
- Skip with reason: prefer row-level `disabled` / `only_os` /
  `requires_env` over runtime `if`s, so `--dump-plugins` shows
  intent.
- Emit `tools/change` after (un)registering so observers stay
  consistent.
