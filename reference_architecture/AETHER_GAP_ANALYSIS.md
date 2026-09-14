# AETHER Gap Analysis — long-running coding agent program (Wave 0)

Supersedes the v0.27-era body below (preserved in git history). Current code: v0.29.1.
Source: `AETHER_RECONNAISSANCE.md`. For each task domain (§2–25): CURRENT / GAP / TARGET.

## Runtime & loop (§2–4)

- CURRENT: `Agent::run` (1295 lines) owns the whole lifecycle; plan→execute→verify works; roles separated via `resolve()`.
- GAP: monolithic control; cancel is iteration-boundary-only; silent executor→controller fallback; TUI composition drift.
- TARGET: phase-extracted runtime modules; cancellable boundaries; explicit NotConfigured; unified composition.

## Provider/model (§5–6)

- CURRENT: generic `openai_compatible` only; model-in-body; blocklists; live validation+fingerprints; UI supports arbitrary endpoints.
- GAP: Minimax never live-validated; unknown `max_tokens` vs `max_completion_tokens` acceptance; reasoning-field tolerance unverified; no preset template.
- TARGET: MiniMax-M3 verified over the generic client; only live-proven adaptations, via generic mechanisms; preset template as data.

## Context (§7–9,13)

- CURRENT: coarse estimator live; rich `budget.rs` dead; `ContextManager` inert; structured transactional checkpoints live with dynamic tail.
- GAP: estimator ignores tools/images/usage; no per-component diagnostics; states exist in triplicate (thresholds/health/budget).
- TARGET: budget estimator wired into preflight; single diagnostics shape `{estimated,reserve,limit,remaining,state}`; states `safe/warning/critical/compaction_required`.

## Compaction & checkpoint (§8,10,11)

- CURRENT: generate→validate→persist→rebuild; `Running/Completed/Failed`; checkpoint framed as session data; evidence-gated completion.
- GAP: no canonical facade (two trigger enums); no CLI/TUI manual path; evidence ephemeral.
- TARGET: one `ContextCompactionService`; `--compact`/slash everywhere; persisted evidence summaries.

## Workspace (§14–15)

- CURRENT: sandboxed R/W/replace/list/stat, ranged reads, git shell-out, notify+debounce watcher, diff UI.
- GAP: whole-file loads; no glob; literal-only search; non-git lacks Added/rename; 250ms vs 280ms debounce comment/code mismatch.
- TARGET: fills only as acceptance test demands; fix debounce comment; keep workspace authoritative.

## Tools (§16)

- CURRENT: id/name/desc/schema/permissions/result/errors; seam pipeline; MCP; role toolsets.
- GAP: no timeout/cancellation/session-scope in ToolContext; model discovers via static schemas (fine); large results only truncated (50KB), no external-ref storage.
- TARGET: timeout+cancel+session in ToolContext; overflow results to session-external refs.

## Verification (§10,17)

- CURRENT: implement→verify→replan with LLM3 conclusion; test counts parsed; SonarQube advisory.
- GAP: evidence self-reported prose; findings never auto-enter EvidenceBag; no persistence.
- TARGET: evidence grounded in diffs/logs; persisted summaries; optional findings bridge.

## Skills/memory/sessions (§18–20)

- CURRENT: all three real and well-separated; per-session isolation via sid keys + kv.
- GAP: minor (skill index duplication mind/skills; isolation unverified by test).
- TARGET: verify isolation by test; no architecture change.

## Persistence/recovery (§21)

- CURRENT: sessions/messages/parts/tool_calls/checkpoints/kv/traces; snapshots; analysis store.
- GAP: resume drops transcript; no versioned migrations; FKs unenforced; EvidenceBag lost on restart.
- TARGET: transcript rehydration; `schema_version` + migrations; evidence persistence.

## Events/errors/security/observability (§22–25)

- CURRENT: 19 TaskEventKinds produced; typed provider/tool errors; redaction + sandbox + minimal ACL.
- GAP: frontend ignores task-state; tool errors flat strings at boundary; no diagnostics surface.
- TARGET: rendered progress/verification/compaction; seam-mapped taxonomy; diagnostics minus secrets.

## Testing/docs (§26–29)

- CURRENT: ~330 unit tests; 1 mock integration test; docs/ has 9 guides; CI runs zero tests.
- GAP: no long-running/compaction-persistence/resume/integration coverage; CLI 0 tests; stale arch docs.
- TARGET: §28 acceptance test; regression matrix; CI test workflow; this doc set synchronized.
