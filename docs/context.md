---
title: "AETHER — Project Context"
status: "canonical"
audience: "agent core / context-engine contributors"
summary: "How the agent discovers repo context and assembles prompt context under a token budget; mirrors AGENTS.md/CONTEXT.md model."
---

# Project Context & Context Engineering — `aether`

This file is both **documentation** and a **template** for the repo-level `CONTEXT.md` the
agent auto-discovers (spec §11, §12). The agent reads hierarchical
`AGENTS.md` / `CLAUDE.md` / `AETHER.md` at repo root and per-directory; more specific wins.

---

## 1. Context assembly order (priority, spec §12)

Never dump the repo. Assemble under a hard token ceiling (`context.max_tokens`):

1. Current task
2. Relevant files (progressive discovery, not read-everything)
3. Errors / failures
4. Project instructions (`AGENTS.md`, `AETHER.md`)
5. Relevant memory (from `aether-mind`)
6. Previous decisions / episodic memory
7. Broader repo map (language, framework, pkg mgr, test runner, entrypoints)

---

## 2. Progressive repository discovery

On entering an unfamiliar repo, build a lightweight map — **no full read**:

```
language      → file extensions / pack files (Cargo.toml, package.json, pyproject.toml)
framework     → lockfile + dir heuristics (src/, app/, tests/)
pkg manager   → npm/pnpm/cargo/pip/uv
build system  → Makefile / justfile / turbo.json / cargo
test framework → vitest / pytest / cargo test
entry points  → main / bin / server
config        → .env, *.toml, *.yaml (read keys names only, never values)
docs          → README, docs/
```

---

## 3. Memory layers (spec §9.1)

| Layer | Stored as | Example |
|---|---|---|
| User | graph + kv | "prefers Python", "concise explanations" |
| Project | graph | "FastAPI + PostgreSQL", "pytest" |
| Session | SQLite (ephemeral) | current task state |
| Episodic | graph + temporal edges | past decisions, prior mistakes |
| Semantic | vector index | docs, code excerpts |
| Skills | filesystem `SKILL.md` | reusable procedures (indexed by name+desc only) |

`MEMORY` = what the agent knows. `SKILL` = how it repeats a task. They are never conflated.

---

## 4. Why not just a bigger context window (spec §9.2)

- Lost-in-the-middle degradation on long context.
- Stuffing wastes tokens/latency/cost on every call.
- Flat transcripts can't resolve contradictions structurally.
- `aether-mind` resolves "slow → medium" style updates **once at write time** via temporal
  supersession (§9.6), not on every read.

---

## 5. Hybrid retrieval (spec §9.7)

```
query → understand(entities+intent)
  → vector(usearch) + keyword(tantivy) + graph(1-2 hop) + temporal(valid_from/until)
  → dedupe → weighted score → rerank → truncate to memory_top_k → inject
```

Controller decides *when* to query memory (new task, "as before", ambiguous ref) — never blindly
on every raw call. This bounds latency/cost.

---

## 6. Forgetting (spec §9.8)
- Explicit `/memory forget <id>`.
- Automatic decay: low-confidence + low-importance + never-reinforced after TTL (logged, never silent).
- **Never auto-delete:** user constraints, security facts ("never auto-commit"), pinned memory.

---

## See also
- [skills.md](./skills.md) — skill loading (lazy, like memory)
- [plan.md](./plan.md) — Phase 3 owns `aether-mind`
- [design.md](./design.md) — `--pin` marker for pinned memory
- [architecture.md](./architecture.md) — `aether-mind-*` crate split
