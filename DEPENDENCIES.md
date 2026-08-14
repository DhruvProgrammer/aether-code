# Dependency Ledger

Every crate is justified. No "might be useful later". (spec §30)

| Crate | Used by | Why |
|---|---|---|
| `serde` / `serde_json` | all | Config + provider JSON (de)serialization |
| `toml` | aether-config | Parse `config.toml` |
| `dirs` | aether-config | Locate `~/.aether/config.toml` |
| `reqwest` (json, stream) | aether-models | OpenAI-compatible HTTP transport |
| `async-trait` | aether-models, aether-tools | Object-safe `ModelProvider` / `Tool` traits |
| `futures-util` | aether-models | `BoxStream` for token streaming |
| `tokio` (process, rt) | aether-tools, aether-cli | Async runtime + safe command execution |
| `thiserror` | aether-config, aether-models, aether-tools | Library error types |
| `anyhow` | aether-core, aether-cli, aether-sessions | Application-level errors |
| `clap` (derive) | aether-cli | CLI parsing |
| `tracing` / `tracing-subscriber` | aether-cli | `--debug` observability |
| `rusqlite` (bundled) | aether-sessions | SQLite session/task/checkpoint store (Phase 2, spec §21) |
| `uuid` | aether-sessions | Session IDs |
| `chrono` | aether-sessions | RFC3339 timestamps for log rows |
| `redb` | aether-mind | Embedded graph + kv + vector store (Phase 3, spec §9) |
| `aether-mind` | aether-core, aether-cli | Memory engine + skills + context discovery |

**Rejected for v1:** `ratatui` (replaced by a dependency-free ANSI styling layer in
`aether-cli/src/ui.rs` wired to `docs/design.md` tokens — keeps the binary light and the GNU-only
build safe), `usearch` (deferred — C++ core needs a C++ linker the GNU-only toolchain lacks;
replaced by a pure-Rust brute-force cosine index with the same `VectorStore` surface, spec §9),
`tantivy`, `axum` (Phase 6 server). `git2` rejected in favor of shelling out to the `git` binary
(honors user git config/hooks, spec §12).
