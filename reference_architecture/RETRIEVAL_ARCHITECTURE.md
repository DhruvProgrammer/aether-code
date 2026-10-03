# Retrieval Architecture — multi-stage pipeline

Implements task-spec §17–27. All weights live in `Weights` (configurable +
unit-tested); no hardcoded top-K — the packer fills a token budget.

## 1. Stages (`retrieve.rs`)

```text
request ─▶ QueryProfile ─▶ candidates ─▶ hybrid score ─▶ filter ─▶ rerank
  ─▶ expand relations (budgeted) ─▶ dedupe ─▶ conflict resolve ─▶ pack
```

## 2. Query understanding

`QueryProfile`: task text, intent (`debug | plan | verify | implement`),
entities (quoted strings, CamelCase symbols, `path/to/file.rs`, error
shapes like `E0308` / `FAILED test_foo`), wanted memory types per intent:

| Intent | Boosted types |
|---|---|
| debug | error, bug, fix, implementation, verification, file_insight |
| plan | decision, requirement, dependency, important_fact, plan |
| verify | verification, implementation, error, tool_result |
| implement | plan, decision, file_insight, dependency, episode |

## 3. Hybrid score

```text
score = w_lexical * bm25_norm + w_recency * recency + w_importance * importance
      + w_task * type_match + w_file * file_overlap + w_relation * related
      - stale_penalty - contradiction_penalty - duplication_penalty
```

Signals: FTS5 BM25 (lexical — catches identifiers/error strings embeddings
miss), recency (log-decay, never overriding relevance), importance,
task-type match, file/symbol overlap with the request, 1-hop relation
bonus. Penalties: superseded/stale status, contradicted-by-newer,
near-duplicate content hash.

## 4. Rerank → expand → pack

1. Sort by score, keep candidates above `min_score`, cap at `max_candidates`.
2. Expand 1-hop relations for kept items within `expansion_budget`.
3. Resolve conflicts: drop `superseded_by`-nonempty records unless history
   requested; prefer newer user corrections (§35).
4. Pack into `memory_budget_tokens`: highest score first, prefer `summary`
   when `content` alone exceeds the remaining budget (§26–27).

## 5. Output contract

`Retrieval { memories: Vec<ScoredMemory>, diagnostics: RetrievalDiagnostics }`
where diagnostics carry per-item score breakdown + provenance
(session/task/file/sources) + latency (§56–57). The caller (Agent) renders
memories into the `PromptCompiler` memory slot and drops them when the
budget (existing `ContextManager`) says so — never bottom-truncate (§27).
