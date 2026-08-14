# AETHER — Multi-Agent Subsystem Architecture

Integration design for the multi-agent subsystem (spec: `full_design.md`). This document is the
required `docs/AGENT_ARCHITECTURE.md` (spec §61). It adapts the OpenClaude-style agent
architecture into AETHER **without** redesigning the existing agent.

## 1. Two-LLM architecture (north star)

```
                    USER
                      |
                      v
             SMALL LLM  (Controller / orchestrator)
                      |
        +-------------+-------------+
        |             |             |
        v             v             v
    EXPLORER      PLANNER      DESIGNER
    RESEARCHER
        |             |             |
        +-------------+-------------+
                      |
                SMALL LLM  (synthesis)
                      |
                      v
              BIG LLM  (Implementer / Executor)
                      |
                  IMPLEMENT
                      |
                      v
             +--------+--------+
             |                 |
          TESTER          REVIEWER
          SECURITY REVIEWER
                      |
                      v
               SMALL LLM  (analysis)
                PASS  -> DONE
                FAIL  -> update eng model, replan, BIG LLM again
```

- **SMALL LLM = controller / engineering intelligence.** Plans, explores, routes, synthesizes,
  evaluates. Specialized agents (Explorer, Planner, Designer, Researcher, Tester, Reviewer,
  Debugger, Security Reviewer, Documenter) normally run on the SMALL LLM.
- **BIG LLM = implementation / execution.** The `Implementer` runs on the BIG LLM. It receives a
  structured task built by the SMALL LLM — not the raw conversation.
- This matches AETHER's existing `controller` (small) + `executor` (big) split exactly.

## 2. Agent hierarchy & registry

Agents are specialized workers, not independent systems. Definitions live as TOML files under
`agents/<id>.toml` (loaded by `AgentRegistry`) with built-in defaults so the system works without
files present. Each definition carries:

```
id, name, description, role, when_to_use, system_prompt,
model ("controller" | "executor"), tools (allowlist), disallowed_tools (denylist),
mode ("build" | "plan"), permissions, can_spawn, max_children, timeout_secs, budget
```

`AgentRegistry` supports `register / load / validate / list / find / enable / disable`. Effective
tool set = `allowlist - denylist`. Effective permissions = base policy overridden per agent;
PLAN-mode or read-only agents get `edit/delete/git_commit = Deny` **mechanically** (never by prompt
only).

## 3. Routing & two-LLM enforcement

`AgentRouter` selects agents from the task (keyword match over `when_to_use`/role) and risk level.
Model resolution is enforced in one place:

```
def.model == "executor"  -> BIG LLM (executor provider)
else                     -> SMALL LLM (controller provider)
```

Verification pipeline after implementation: `Tester` + `Reviewer` always; `Security Reviewer` is
added when the task touches auth/security/secrets/network/permissions or risk is high.

## 4. AgentTool / runner

`agents::runner::run_agent` is the execution primitive (conceptually OpenClaude's `AgentTool`).
It builds an `Executor` with the agent's effective tools + effective policy + system prompt, runs
it, and parses the structured `SubagentResult`. It is foreground-only in this phase; background /
resume / message / cancel / worktree isolation are follow-ups (see §9).

## 5. Agent context

`AgentContextBuilder` assembles **only relevant** context per agent: global rules + current mode +
agent role + goal + task + relevant memory + engineering state + constraints + success criteria.
It does not dump the whole conversation.

## 6. Lifecycle

`AgentLifecycle` tracks `CREATED → QUEUED → RUNNING → WAITING → COMPLETED` (plus `FAILED /
CANCELLED / TIMED_OUT / BLOCKED`). Every run has `run_id`, `parent_run_id`, `session_id`,
`task_id`, `loop_run_id`. `LifecycleTracker` enforces `max_depth` (recursion) and `max_children`
to prevent agent explosions.

## 7. Integration with existing systems

- **EngineeringModel (`eng`)**: agents consume/produce facts, unknowns, hypotheses, evidence. The
  Controller updates the model from agent findings each loop (spec §53).
- **Build/Plan modes (`mode`)**: PLAN MODE forces read-only for **every** agent, overriding its
  permission set (spec §27). BUILD MODE runs agents subject to permissions + Karpathy.
- **Memory (`aether-mind`)**: relevant memories are retrieved and injected into agent context.
- **Skills**: the `karpathy-guidelines` skill remains the default behavior; agents inherit it.
- **LoopEngine**: remains the OUTER orchestration layer; the agent subsystem runs inside it.

## 8. Diagram

```
USER ─▶ Controller(SMALL)
            │
            ├─ Explorer(SMALL)  ──▶ findings
            ├─ Planner(SMALL)   ──▶ plan
            ├─ Designer(SMALL)  ──▶ design   (complex tasks)
            ├─ synthesis
            │
            └─▶ Implementer(BIG) ──▶ code
                        │
                  Tester(SMALL) + Reviewer(SMALL) [+ Security(SMALL)]
                        │
                  Controller evaluates ─▶ DONE | REPLAN
```

## 9. Phase status (this implementation)

| Phase | Status |
|-------|--------|
| AgentDefinition / Registry / Router / ContextBuilder / Lifecycle | ✅ implemented |
| 10 agents defined (Planner, Designer, Explorer, Researcher, Implementer, Tester, Reviewer, Debugger, Security Reviewer, Documenter) | ✅ defined |
| Two-LLM routing enforced (Implementer→BIG, others→SMALL) | ✅ implemented |
| Explorer-before-plan + router-driven verification in `Agent::run` | ✅ implemented |
| EngineeringModel integration from agent findings | ✅ implemented |
| Karpathy + Build/Plan mode integration | ✅ (from prior phase) |
| Foreground/background, resume, message, cancel, parallel, worktree isolation | ⏳ follow-up (Phase 4) |
| Tracing / events / CLI inspection, budgets enforcement depth | ⏳ follow-up (Phase 6) |
| `AgentTool` as a controller-invokable tool (controller spawns via tool call) | ⏳ follow-up (Phase 5 deep) |
