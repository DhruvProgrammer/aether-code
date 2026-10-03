# Memory Schema — `aether-memory` records

Adapted from task-spec §11 to AETHER's session/task model. Field names are
the Rust struct fields in `aether-memory/src/types.rs`.

## 1. `MemoryRecord`

| Field | Type | Notes |
|---|---|---|
| `id` | String (uuid) | primary key |
| `project_id` | String | repo fingerprint / path; scope boundary (§39–40) |
| `session_id` | Option<String> | `None` = project-scoped, visible across sessions |
| `task_id` | Option<String> | episode grouping |
| `mem_type` | MemoryType | §12 enum, validated (no free-form strings) |
| `title` | String | one-line, self-contained (§23) |
| `content` | String | contextualized body, understandable in isolation |
| `summary` | Option<String> | compact form for packing under pressure |
| `source_ids` | Vec<String> | message/tool/trace ids (provenance, §57) |
| `files` | Vec<String> | workspace paths; revalidated at retrieval (§34) |
| `symbols` | Vec<String> | `Type::method`, `function`, etc. (§19) |
| `tags` | Vec<String> | lowercase keywords |
| `importance` | f32 0..1 | correction=0.95, decision=0.8, trivia filtered |
| `confidence` | f32 0..1 | verified tool evidence > model inference |
| `status` | MemoryStatus | candidate/active/superseded/stale/archived/deleted (§42) |
| `supersedes` | Vec<String> | ids this record replaces |
| `superseded_by` | Vec<String> | ids replacing this one |
| `parent_id` | Option<String> | episode hierarchy (§16) |
| `related_ids` | Vec<String> | typed edges (relation in `relations` table) |
| `created_at` / `updated_at` | i64 (unix) | recency + TTL math |

## 2. `MemoryType` (minimum set, §12)

```text
requirement constraint decision plan task episode implementation
file_insight tool_result error bug fix verification dependency
user_correction important_fact open_question
```

`user_correction` pins `importance >= 0.9` and triggers supersession of
contradicted records (§13–14).

## 3. `Relation`

```text
supersedes | superseded_by | derived_from | contradicts | depends_on | related_to
```

Stored in a `relations(from_id, to_id, relation)` table; traversal is
1-hop by default with a caller-supplied expansion budget (§24).

## 4. Storage tables (`store.rs`)

```text
memories(id PK, project_id, session_id, task_id, mem_type, title, content,
         summary, source_ids JSON, files JSON, symbols JSON, tags JSON,
         importance, confidence, status, parent_id, created_at, updated_at,
         content_hash)
memories_fts(title, content, tags)   FTS5 lexical index (BM25 ranking)
relations(from_id, to_id, relation)
```

`content_hash` (fnv1a of normalized content) backs dedup (§59).
No raw secrets are stored: the extractor redacts before insert (§60).
