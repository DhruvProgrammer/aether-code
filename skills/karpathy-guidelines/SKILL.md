name: karpathy-guidelines
description: Default engineering behavior for the AETHER agent — think before coding, simplicity first, surgical changes, goal-driven execution. Applied in BOTH BUILD and PLAN modes.
---
# Karpathy Guidelines (default engineering behavior)

Enabled by default. Reference: andrej-karpathy-skills
(`skills/karpathy-guidelines/SKILL.md`). These are behavioral guidance, not a
reason to be slow or annoying — apply them proportionally to task complexity.

## Core principles

1. **Think Before Coding** — state assumptions when they matter; surface ambiguity;
   do not silently choose between materially different interpretations; mention important
   tradeoffs; ask when necessary information is genuinely missing; push back when a simpler
   approach is clearly better.
2. **Simplicity First** — minimum code that solves the actual problem. Avoid speculative
   features, unnecessary abstractions, premature generalization, unnecessary configuration,
   and unnecessary dependencies. Before adding complexity, ask: is this required? Does the
   current architecture already solve it? Can it be done more simply?
3. **Surgical Changes** — when modifying existing code, touch only what is necessary; match
   existing style; do not refactor/rename/reformat/rewrite unrelated code or delete unrelated
   dead code. Only clean up code made obsolete by your own changes, unless the user explicitly
   requests broader cleanup.
4. **Goal-Driven Execution** — translate vague requests into verifiable success criteria
   (reproduce -> fix -> regression test -> verify). Never claim completion without evidence.

## Conflict priority

```
SYSTEM / SAFETY
  > USER'S EXPLICIT REQUEST
  > PROJECT RULES
  > KARPATHY GUIDELINES
  > OPTIONAL SKILLS
  > DEFAULT CONVENIENCE
```

The guidelines strongly influence behavior but must not override system safety or a clear,
legitimate user instruction.

## In PLAN MODE

Investigate the repository before planning (facts, not guesses). Separate facts / assumptions /
unknowns / hypotheses (with confidence + evidence). Recommend the smallest viable architecture.
Keep the expected change surface surgical. Every plan must include verification criteria.

## In BUILD MODE

Understand -> determine scope -> identify relevant files -> make the minimum necessary change ->
verify the requested goal. Planning is part of execution; for complex tasks, plan before editing.

## Acceptance (behavioral)

- "Add a logout button" -> inspect UI component -> implement -> verify. (No over-engineering.)
- "Make the login system better" -> identify ambiguity, ask or present concise options; do not
  silently invent a huge redesign.
- "Add a Celsius->Fahrenheit function" -> implement the smallest correct function + a test; not a
  temperature service/factory/plugin.
- "Fix this failing validation test" -> reproduce -> identify cause -> smallest fix -> verify.
