# Architecture Decisions — long-running coding agent program

Record: `{ID, status: proposed|accepted|deprecated, context, decision, consequences}`.
Append-only; never rewrite history — supersede with new entries.

## ADR-001 — Wrap, don't duplicate, the compactor (accepted, Wave 0)

Context: SessionCompactor is transactional and tested; task §12 demands one canonical service.
Decision: `ContextCompactionService` is a facade over `SessionCompactor`; manual + auto call it.
Consequences: no second implementation can drift; existing 14 checkpoint tests keep covering the core.

## ADR-002 — Generic Minimax, verified live (accepted, Wave 0)

Context: Minimax publishes OpenAI-compatible Chat Completions (`api.minimax.io/v1`, Bearer, model-in-body).
Decision: no Minimax-specific types/clients/routers. Ship config + verification; adapt the generic client only on live-proven need.
Consequences: §5/§6 collapse to validation + preset data unless M3 proves otherwise.

## ADR-003 — No-fallback invariant is load-bearing (accepted, Wave 0)

Context: explicit user model assignment is the product contract.
Decision: `resolve()` silent fallback is removed in Wave 2 with explicit NotConfigured surfacing; all 45 gateway tests + no-routing tests stay green throughout.
Consequences: any provider work must fail loudly, never reroute.

## ADR-004 — Decomposition reuses, never rewrites, domain logic (accepted, Wave 0)

Context: `Agent::run` is monolithic but its plan→verify semantics are tested strengths.
Decision: extract phase modules around existing `TaskStateMachine`/`LoopEngine`/`EvidenceBag`/compactor; `!Sync` rusqlite bounds the design (current-thread runtime stays).
Consequences: Wave 2 is refactoring risk, not behavior risk; core test count must not drop.

## ADR-005 — Evidence must be grounded and persisted (proposed, Wave 0)

Context: statuses today derive from subagent prose; EvidenceBag is ephemeral.
Decision (proposed): Pass/Replan verdicts require diff/log/test-output refs; per-session evidence summaries persist in kv.
Consequences: Wave 5; negative tests against fabricated statuses.
