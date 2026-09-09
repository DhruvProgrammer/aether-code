---
name: software-engineering
description: Spec-driven software engineering process (PRD, research, architecture, design, acceptance, tasks, implementation, testing, review, docs) plus Karpathy-style behavioral rules. Loaded dynamically per task — never pasted whole into the system prompt.
---

# Software Engineering Skill

Authoritative source for AETHER's detailed software-engineering behavior.
Split from `source/AI_Software_Engineering_Master_SKILL.md` (verbatim sections).

**Do not paste this skill into the system prompt.** Load only relevant
sections per task via the Skill Retriever (`aether-skills` crate).

## Module layout

- `<section>.md` — full verbatim section text
- `<section>.compact.md` — compressed operational rules (same meaning, lower cost)
- `index.json` — metadata: keywords, dependencies, always-load flags
- `source/` — original authoritative skill (never edited by the retriever)

## Sections

core, context, modes, requirements, research, reconnaissance, architecture,
technical-design, acceptance, tasks, implementation, testing, done, security,
database, api, dependencies, errors, observability, adr, documentation,
ambiguity, existing-project, refactoring, vertical-slice, project-start,
communication.
