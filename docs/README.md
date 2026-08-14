---
title: "AETHER — Documentation Index"
status: "canonical"
audience: "everyone"
summary: "Entry point for the aether agent docs: plan, context, skills, architecture, config, roadmap, design."
---

# AETHER — Docs Index

`aether` is a **Rust**, **OpenAI-API-compatible** coding agent with an embedded
`aether-mind` memory engine, shipped as a single light-weight Windows `.exe`.
This folder is the planning/execution set referenced by the agent itself (spec §11).

## Reading order (recommended)
1. [design.md](./design.md) — minimalist Pantone light UI + SEO-structured markdown rules
2. [plan.md](./plan.md) — phase-gated build plan, OpenAI-compatible first
3. [architecture.md](./architecture.md) — Cargo workspace crate map
4. [context.md](./context.md) — context assembly & memory layers
5. [skills.md](./skills.md) — lazy skill system
6. [config.md](./config.md) — `config.toml` reference
7. [roadmap.md](./roadmap.md) — phases + `.exe` packaging

## Quick facts
- **Language:** Rust (memory-safe, single static binary)
- **API:** 100% OpenAI-compatible (`/v1/chat/completions` + `/v1/embeddings`)
- **Memory:** embedded `redb` graph + `usearch` vector, hybrid retrieval
- **UI:** minimalist light TUI (Pantone-anchored, see design.md)
- **Target:** ≤ 60 MB PSS (embeddings off), single `.exe`, no install-time runtime

## References studied
- [opencode](https://github.com/anomalyco/opencode) — `AGENTS.md`/`CONTEXT.md`, Windows `.exe`
- [jcode](https://github.com/1jehuang/jcode) — RAM efficiency, OpenAI-compatible providers, memory
- [grok-build](https://github.com/xai-org/grok-build) — Rust TUI, composition-root binary

> Implement independently. Do not copy proprietary code or reproduce branding (spec §33).

## See also
Each doc ends with a `## See also` block linking its siblings.
