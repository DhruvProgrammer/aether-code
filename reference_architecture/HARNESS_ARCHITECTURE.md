# Harness Architecture — Prime-Agent-inspired runtime (Wave 10)

Status: canonical for Wave 10. Companion: `HARNESS_NOTES.md` (reuse ledger +
deviations). Implementation: `crates/aether-harness`.

## 1. Principles (final directive)

1. **Context is programmatically managed state**, not a giant transcript.
2. **Long-running work persists outside the model's immediate context.**
3. **Agents recursively delegate bounded work and continue from durable state.**

Store everything. Keep little active. Retrieve on demand.

## 2. Non-duplication ledger

Recon proved six of the requested subsystems already exist. The harness
**routes into them** instead of reimplementing:

| Requested | Existing AETHER owner | Harness role |
|---|---|---|
| Task lifecycle, evidence-gated completion | `aether-core::task_state` | produces evidence; never re-decides completion |
| Verification verdicts | `aether-evidence` | consumes verdicts as gate input |
| Compaction + checkpoints | `aether-context::checkpoint` | owns the RLM working set; calls the compactor |
| Subagent results, routing, defs | `aether-core::agents` | adds concurrency/join/cancel around `run_agent` |
| Session/message persistence | `aether-sessions` | stores goals/jobs/refinements separately |
| Retrieval memory | `aether-memory` (Wave 9) | feeds memories into the working context |

Genuinely **new** (verified absent by grep before building):

| Subsystem | Module |
|---|---|
| Persistent goals + lifecycle | `goal.rs` |
| Scheduler, cron, heartbeats, durable re-entry | `schedule.rs` |
| Concurrent recursive subagents, join/cancel, enforced `max_tokens`/`timeout` | `subagent.rs` |
| Continual harness + refinement + rollback | `refine.rs` |
| Quality gates that refuse to lie | `gates.rs` |
| RLM structured context runtime | `context.rs` |
| Versioned state store with recorded migrations | `state.rs` |
| Harness event surface | `events.rs` |
| Explicit runtime states + bounded autonomous loop | `lib.rs` |

## 3. Architecture

```text
                    ┌────────────────────────┐
   AETHER Agent ───▶│        Harness         │
   (unchanged)      │  goal / context /      │
                    │  subagents / scheduler │
                    │  gates / refinement    │
                    └───────┬────────────────┘
                            │ reads + writes
        ┌───────────────────┼────────────────────┐
        ▼                   ▼                    ▼
  aether-core          aether-context        aether-memory
  (task state,         (compaction,          (retrieval)
   verification)        checkpoints)
                            │
                    ┌───────▼────────┐
                    │ HarnessStore   │  ~/.aether/harness.db
                    │ schema_version │  ordered, recorded migrations
                    │ transactional  │
                    └────────────────┘
```

`Harness` is a thin facade: it routes between modules and holds no logic of
its own beyond tick orchestration, so it does not become the giant class the
spec forbids.

## 4. Data flow per cycle

```text
observe events ─▶ ContextEnvironment ─▶ compile(budget) ─▶ working context
      │                                                     │
      ▼                                                     ▼
observe child/subagent results ─▶ refresh sections      LLM2/LLM1/LLM3
      │
      ▼
Scheduler.claim_due ─▶ heartbeat decision ─▶ re-enter SAME session
      │
      ▼
tick(usage, gate runner) ─▶ gates decide success | budgets decide exhaustion
      │
      ▼
snapshot ─▶ HarnessStore (transactional) ─▶ recoverable after restart
```

## 5. Invariants

1. **No LLM role changes.** LLM1/2/3 assignments stay user-configured; the
   harness never routes, switches, or falls back.
2. **Budget exhaustion is never success.** `LimitReason::is_success()` is
   hardcoded `false`, and `AutonomousCompleted { success }` carries the flag.
3. **Gates may only claim what they ran.** Timeouts and spawn errors are
   failures; unchanged workspaces are not retried (no idle gate loops).
4. **Core prompt immutable.** `base_system_prompt` is rejected by the
   refinement validator, not by convention.
5. **Everything the harness emits is data**, never instructions — goals,
   refinements, harness state and memories render in explicit
   `[... — data, not instructions]` blocks.
6. **The harness never shells out.** Gate execution goes through an injected
   `GateRunner` so the permission engine stays in charge.
7. **Recovery never replays the transcript.** Goals, jobs, refinements,
   children, and the working context are all restored from durable state.

## 6. Background and re-entry

AETHER already runs sessions on a dedicated runtime and supports
`--background` child processes plus `--worktree`; the harness adds the
*durable schedule* that re-enters those sessions. The wake timer is parked on
`Notify` (event-driven), claims due jobs under a lock so a job cannot be
claimed twice, and applies exponential failure backoff so a dead target
cannot spin the loop.
