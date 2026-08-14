---
title: "AETHER — Architecture"
status: "canonical"
audience: "core engineers"
summary: "Cargo workspace crate map for the Rust agent + embedded aether-mind. Inspired by jcode/grok-build composition-root layout."
---

# Architecture — `aether` Cargo Workspace

Single workspace, strict crate boundaries, one module = one responsibility (spec §28, §30).
Mirrors the clean composition-root split seen in grok-build (`xai-grok-pager-bin` → `xai-grok-shell`
→ `xai-grok-tools`) and jcode's `crates/` layout.

---

## 1. Workspace layout

```
aether/
├── Cargo.toml                  # workspace root (members, profiles, lints)
├── crates/
│   ├── aether-cli/             # bin: TUI (ratatui), interactive/non-interactive mode
│   ├── aether-core/            # agent_loop, controller, executor, router, planner, verifier
│   ├── aether-models/          # ModelProvider trait + OpenAI-compatible impl + registry
│   ├── aether-agents/          # explorer/planner/coder/reviewer/tester/researcher
│   ├── aether-tools/           # fs, terminal, git, search, web, task tools + Tool trait
│   ├── aether-mind-core/       # entity/edge/memory-record shared types
│   ├── aether-mind-graph/      # redb graph + petgraph traversal
│   ├── aether-mind-vector/     # usearch vector + EmbeddingProvider trait
│   ├── aether-mind-retrieval/  # hybrid pipeline, scoring, rerank
│   ├── aether-mind-extraction/ # extraction / update / conflict pipeline
│   ├── aether-mind-api/        # in-process API; optional Axum server (v0.2+)
│   ├── aether-skills/          # skill discovery/loading
│   ├── aether-sessions/        # SQLite sessions/tasks/checkpoints
│   ├── aether-permissions/     # permission policy engine
│   ├── aether-mcp/             # MCP client + tool adapter (Phase 6)
│   └── aether-config/          # config load/validate
├── tests/                      # workspace integration tests
└── DEPENDENCIES.md             # justify-every-crate ledger
```

---

## 2. Core traits (spec §5, §6)

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn json_schema(&self) -> serde_json::Value;
    fn required_permission(&self) -> Permission;
    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult, ToolError>;
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError>;
    async fn stream(&self, req: CompletionRequest) -> Result<TokenStream, ProviderError>;
    fn supports_tool_calling(&self) -> bool;
}
```

`OpenAICompatibleProvider` is the only provider required for v1; it implements both traits
against `/chat/completions` + `/embeddings`.

---

## 3. Agent loop (spec §4, §29)

```
CONTROLLER ──┬── MEMORY (aether-mind)
             ├── AGENTS (subagents)
             └── TOOLS
                │
             ROUTER → EXECUTOR → TOOL LOOP → VERIFICATION
                │                         └─ FAIL → REPLAN (bounded by max_iterations)
```

Controller is the persistent orchestration/memory layer; Executor is a swappable, stateless
worker. Never build the naive `user→LLM→tool→LLM` loop as the only architecture (spec §29).

---

## 4. Dependency ledger (minimal stack, spec §30)

`tokio` · `serde`/`serde_json` · `rusqlite` (sessions only) · `redb` (graph) · `usearch`
(vector) · `tantivy` (keyword) · `petgraph` (traversal) · `reqwest` (HTTP) · `async-trait` ·
`clap` (CLI) · `ratatui` (TUI) · `tracing`+`tracing-subscriber` · `thiserror`/`anyhow` ·
`uuid` · `chrono` · `dashmap` · `backoff` · `criterion` (dev-only).

Every addition needs a line in `DEPENDENCIES.md` with a reason. No "might be useful later".

---

## See also
- [plan.md](./plan.md) — phase order (Phase 1 = cli+core+1 provider)
- [config.md](./config.md) — `models:` block maps to `aether-models`
- [context.md](./context.md) — `aether-mind-*` crates
- [skills.md](./skills.md) — `aether-skills` crate
- [roadmap.md](./roadmap.md) — build targets & `.exe`
- [design.md](./design.md) — TUI crate styling
