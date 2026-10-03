# Implementation Plan — long-running coding agent program (Wave 0)

Supersedes the v0.27-era body (git history preserves it). Waves run critical-path first.
Status legend: `TODO` / `DOING` / `DONE` / `BLOCKED(key)`. Update statuses as waves land.

## Wave 0 — Docs + baseline (DOING)

| Component | Current | Target | Files | Deps | Risks | Tests | Status |
|---|---|---|---|---|---|---|---|
| Reconnaissance | tribal knowledge | AETHER_RECONNAISSANCE.md | reference_architecture/ | surveys | staleness | n/a (record) | DOING |
| Gap/plan/decisions | stale v0.27 docs | rewritten set, this file | reference_architecture/ | recon | scope creep | n/a | DOING |
| Baseline | unknown green | recorded results | — | toolchain | long build | cargo test --workspace, tsc | TODO |

## Wave 1 — Runtime survival

| Component | Current | Target | Files | Deps | Risks | Tests | Status |
|---|---|---|---|---|---|---|---|
| Tool timeouts | unbounded awaits | timeout+cancel in ToolContext; bounded exec/MCP/git | tools/lib.rs,mcp.rs,git.rs,executor.rs | tokio time/select | killing MCP mid-handshake | timeout fires; cancel aborts; no hang | TODO |
| Resume | plan+engineering only | transcript rehydration via get_messages | run_task.rs,agent_loop.rs,executor.rs | sessions | context blowout on huge histories (cap+compact) | resume continues same task | TODO |
| Auto snapshots | triggers never fire | PreDanger/PreCompaction auto-snapshot | executor.rs,checkpoint flow | snapshots | disk growth (cap chain) | trigger→snapshot exists | TODO |
| Budget wiring | dead estimator | preflight uses budget::build + diagnostics event | context/*,executor.rs | — | estimator drift vs providers | diagnostics shape test | TODO |

## Wave 2 — Runtime decomposition

| Component | Current | Target | Files | Deps | Risks | Tests | Status |
|---|---|---|---|---|---|---|---|
| Phase modules | 780-line run() | task/context/tool/verification/session/model runtime modules reusing existing types | core/agent_loop.rs split | all above | Send-ness (!Sync conn) | existing 88 core tests stay green | TODO |
| Explicit binding | silent fallback | NotConfigured surfacing | agent_loop.rs resolve() | gateway | desktop Exit{2} handling | fallback-removal test | TODO |
| TUI parity | drifted minimal Agent | unified composition builder | cli/tui.rs,run_task.rs | runtime | TUI bloat | TUI boots host/offline | TODO |

## Wave 3 — Provider + Minimax (BLOCKED(user API key at run time))

| Component | Current | Target | Files | Deps | Risks | Tests | Status |
|---|---|---|---|---|---|---|---|
| M3 validation | unverified | live chat/tools//models/errors vs MiniMax-M3 global | none (config only) | key | key handling (env-only, never logged) | live run once + mock parity | TODO |
| Body compat | unknown | adopt max_completion_tokens mapping + reasoning tolerance ONLY if live-proven | models/openai.rs | gateway tests | Minimax-specific leakage → keep generic | unit tests for any adapter change | TODO |
| Preset | manual entry | generic preset template data (Minimax first entry) | config/ui data | desktop | hardcoded-logic creep → data only | preset loads | TODO |

## Wave 4 — Compaction canonical + /compact

Single ContextCompactionService facade over SessionCompactor; CLI `--compact`, TUI slash, desktop keeps compact_session; all call the facade. Files: context/*, cli/*, desktop main.rs. Tests: manual==auto path identity; transactional failure keeps old state.

## Wave 5 — Verification grounding + persistence

Persist evidence summaries per session; ground Pass/Replan in diffs+logs; optional SonarQube→Evidence bridge. Files: evidence/*, sessions kv, agent_loop verify phase. Tests: restart preserves verification state; no fabricated statuses (negative tests).

## Wave 6 — Workspace/skills/memory fills

Glob search, non-git Added tracking, chunked large-file reads — ONLY as Wave 8 demands. Skill/mind: isolation test, index-dedup check. No architecture changes planned.

## Wave 7 — Events/errors/security/observability

Render task-state (progress/verification/compaction/health) in UI; seam-mapped typed tool errors; redaction audit; diagnostics events. ACL unchanged. Tests: event→UI mapping; secret-leak negative tests.

## Wave 8 — Acceptance + regression + CI

§28 long-session integration test (mock providers; 18 steps incl. forced compaction→continuation→survival proof); §29 matrix (unit/integration/tsc/vite/build/validation/session-switch/tools/changes/compaction//compact); new CI workflow running cargo test + tsc (repo currently runs none).

## Wave 9 — Context Memory Engine (DOING)

New `aether-memory` crate; arch in `MEMORY_ARCHITECTURE.md` / `MEMORY_SCHEMA.md` /
`RETRIEVAL_ARCHITECTURE.md` / `MEMORY_EVALUATION.md`.

| Component | Current | Target | Files | Deps | Risks | Tests | Status |
|---|---|---|---|---|---|---|---|
| Data model | free-form `Mind` nodes | typed `MemoryRecord` + relations | memory/types.rs | — | over-categorization | schema/type tests | DOING |
| Store | redb blobs, no FTS | `MemoryStore` trait + SQLite/FTS5 | memory/store.rs | rusqlite (already in tree) | FTS5 availability | crud/search tests | DOING |
| Extraction | LLM-only, opt-in | rule-based chunker + critical fast-path | memory/extract.rs | — | trivia pollution | boundary/type tests | DOING |
| Retrieval | once-per-task concat | hybrid + rerank + pack per planning call | memory/retrieve.rs, engine.rs | — | weight tuning | buried-info test | DOING |
| Integration | static memory string | engine in `Agent::run` + compiler slot | core/agent_loop.rs | memory crate | prompt drift | existing 88 core tests green | TODO |
| Eval | none | scenario matrix + 100-turn benchmark | memory tests | — | synthetic bias | §48 benchmark | TODO |
