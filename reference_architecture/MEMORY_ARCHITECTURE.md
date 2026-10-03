# Memory Architecture — Context Memory Engine (Wave 9)

Status: canonical for Wave 9. Companion docs: `MEMORY_SCHEMA.md`,
`RETRIEVAL_ARCHITECTURE.md`, `MEMORY_EVALUATION.md`.

## 1. Why (research basis)

Long-context models degrade when relevant facts are buried in long prompts
("Lost in the Middle", TACL 2024) and reliability decays as context grows
("context rot"). The fix is not a bigger window: keep a **small active
working set** and retrieve the rest from an external store.

## 2. Layer model (§4–10 of the task spec)

```text
Layer 0  Active Working Context   what is sent to the model (small)
Layer 1  Recent Session Memory    last meaningful events, current files/error
Layer 2  Task / Episode Memory    grouped work units with state + outcome
Layer 3  Project Semantic Memory  durable facts, decisions, constraints
Layer 4  Code / Workspace Memory  file/symbol knowledge (workspace wins)
Layer 5  Raw Historical Record    complete session log (never destroyed)
```

Mapping onto existing AETHER parts (reuse, don't rebuild):

| Layer | Built from |
|---|---|
| 0 | `PromptCompiler` (aether-skills) output + retrieved memories |
| 1 | `SessionStore::get_messages` tail + `tool_calls` table |
| 2 | New `MemoryRecord{type=episode}` + parent/child relations |
| 3 | New typed records in `MemoryStore` (replaces free-form `Mind` nodes over time) |
| 4 | File/symbol records with workspace paths; workspace is source of truth |
| 5 | `sessions/messages/parts/tool_calls/traces` tables (already complete) |

## 3. New crate: `aether-memory`

Pure-logic crate (no provider, no UI) so the engine is testable without a
model key:

```text
aether-memory/
  types.rs      MemoryType / MemoryStatus / Relation / MemoryRecord
  store.rs      MemoryStore trait + SqliteMemoryStore (FTS5 lexical)
  extract.rs    semantic chunking: messages -> MemoryRecord candidates
  retrieve.rs   query understanding + hybrid score + rerank + packing
  engine.rs     MemoryEngine facade: observe() + retrieve() + diagnostics
```

`aether-mind` (graph/kv/vector) stays for cross-session recall; the typed
store is the per-project/session working memory. No duplication: raw history
stays in `aether-sessions`, embeddings stay behind `ModelProvider`.

## 4. Data flow

```text
messages/tool events ──observe()──▶ MemoryStore (incremental, §45)
                                            │
current request ──retrieve()──▶ packed memories ──▶ PromptCompiler.memory slot
                                            │
                              diagnostics (provenance, scores, latency)
```

Extraction runs inline for critical records (user corrections, decisions)
and deferred otherwise (§36); retrieval runs per Controller planning call,
not once per task.

## 5. Invariants

1. Store everything; keep little active; retrieve intelligently (§64).
2. Retrieved memory is DATA — never instructions (§38).
3. Workspace always wins over memory (§34); stale records are marked, not
   silently merged (§35).
4. No secret (key/token/password) is ever embedded or indexed (§60).
5. LLM1/2/3 roles unchanged; LLM2 owns semantic extraction (§53–54).
