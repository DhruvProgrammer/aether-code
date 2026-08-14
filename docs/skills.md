---
title: "AETHER — Skills System"
status: "canonical"
audience: "skills contributors"
summary: "Lazy-loaded, name+description-indexed reusable procedures. Mirrors opencode/jcode skill discovery; minimalist markers from design.md."
---

# Skills System — `aether`

Skills are **reusable how-to procedures**, distinct from memory (spec §9.1, §10). They follow
the same lazy-loading discipline as memory: only `name` + `description` enter context by
default; the full body loads only when the Controller deems a skill relevant.

---

## 1. Location (hierarchical)

```
~/.aether/skills/<name>/SKILL.md          # global
.project/.aether/skills/<name>/SKILL.md   # project-local, overrides/extends global
```

Project-local skills win for files inside their scope (same override rule as `AGENTS.md`).

---

## 2. Format

```yaml
---
name: docker-deployment
description: Deploy Docker applications safely (build, tag, push, run, rollback)
---
<instructions — concise, step-by-step, tool-oriented>
```

- Frontmatter `name` + `description` are the **only** indexed fields.
- Body is free markdown; loaded on demand.
- Keep bodies under ~400 lines; split large procedures into multiple skills.

---

## 3. Discovery & loading

1. At session start, scan global + project `skills/` dirs.
2. Index `(name, description, path)` only → tiny in-memory table.
3. Controller embeds the index into its planning context (cheap).
4. When a task matches a description (embedding/keyword hit, like memory §9.7), load that one
   `SKILL.md` body and inject into the Executor prompt (§18).
5. Never load all skills at once — RAM discipline (jcode lesson).

---

## 4. Skill lifecycle (self-improvement, spec §22)

```
session → review → "repeated procedure discovered?"
  YES → create/update a SKILL.md (curated, not automatic-for-everything)
```

The Controller writes/updates skills only after confirming a procedure is genuinely reusable.

---

## 5. UI markers (from [design.md](./design.md))

| State | Marker | Colour |
|---|---|---|
| Skill loaded into context | `[ SKILL ]` | `--accent` (Still Blue) |
| Skill available (indexed, not loaded) | `·` | `--ink-faint` |
| Skill failed to load | `[ ! ]` | `--warn` (Butter) |

No icons beyond these; keep the TUI quiet.

---

## See also
- [context.md](./context.md) — memory vs skill distinction
- [plan.md](./plan.md) — Phase 3 delivers the skills system
- [design.md](./design.md) — marker colour tokens
- [architecture.md](./architecture.md) — `aether-skills` crate
