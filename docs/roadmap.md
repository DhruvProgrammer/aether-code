---
title: "AETHER — Roadmap & Packaging"
status: "canonical"
audience: "release engineering"
summary: "Phase gates plus the concrete path to a single static Windows .exe, OpenAI-compatible, minimal RAM footprint."
---

# Roadmap & `.exe` Packaging — `aether`

Phases keep the project runnable after each (spec §32). The packaging target is a **single
static `aether.exe`** for Windows, modeled on opencode's desktop `.exe` and grok-build's
prebuilt Windows binary.

---

## 1. Phase gates

| Phase | Scope | Done when |
|---|---|---|
| 1 | CLI + core + 1 OpenAI provider + fs/term tools | compiles, `/exe` completes 1 live task |
| 2 | permissions, planning mode, git, SQLite sessions | `cargo test -p aether-permissions` green |
| 3 | `aether-mind` MVP + skills + `AGENTS.md` discovery | hybrid retrieval test passes |
| 4 | subagents (explorer/planner/coder/reviewer/tester/researcher) | structured JSON handoff tested |
| 5 | compaction, cost routing, rollback, observability, **Pantone TUI** | clippy clean, TUI on light theme |
| 6 | MCP client, `aether-mind` as MCP server, server/cloud modes | research only |

---

## 2. Windows `.exe` build

```powershell
# Prereqs (choco)
choco install rust msys2 -y
rustup target add x86_64-pc-windows-msvc

# Release build (single binary)
cargo build --release -p aether-cli --target x86_64-pc-windows-msvc
# artifact: target/x86_64-pc-windows-msvc/release/aether.exe
```

- **Static, no runtime deps.** No Node/Python/VC++ redis required at install (like jcode).
- Strip symbols + `panic = "abort"` in `[profile.release]` for smaller footprint.
- Default memory dir: `%USERPROFILE%\.aether\memory\` (redb + usearch mmap files).

### Installer (optional)
- InnoSetup script → `aether-setup-x64.exe`.
- Or `scoop` manifest (opencode pattern):
  ```json
  { "version":"0.1.0", "architecture": { "64bit": { "url":"aether.exe" } } }
  ```

---

## 3. RAM/footprint budget (differentiator)

Target (1 session, measured PSS, light theme):

| Tool | Target PSS |
|---|---|
| **aether (embeddings off)** | ≤ 60 MB (beat jcode baseline 167 MB) |
| aether (full mind) | ≤ 220 MB |
| OpenCode (for contrast) | ~371 MB |

Levers: lazy skill/memory load, `redb`+`usearch` mmap (no server process), no GC, single binary.

---

## 4. CI (spec §31)
- `cargo test` per crate; providers **mocked** (no live keys).
- `cargo clippy --all-targets` must be clean.
- Build the `.exe` on `windows-latest` runner; smoke-test with a mock provider.
- Build matrix: `ubuntu` + `windows-msvc`. macOS best-effort.

---

## See also
- [plan.md](./plan.md) — phase rationale
- [architecture.md](./architecture.md) — crate ownership per phase
- [config.md](./config.md) — `aether init` defaults
- [design.md](./design.md) — light TUI theme shipped in Phase 5
- [context.md](./context.md) — memory store location
