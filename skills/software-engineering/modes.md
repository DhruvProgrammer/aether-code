# 3. Process Modes

Choose the smallest process that safely fits the change.

## 3.1 Fast Mode

Use for:

- typo fixes,
- isolated documentation edits,
- tiny configuration changes,
- trivial one-line fixes.

Workflow:

```text
Inspect
→ Change
→ Focused Validation
→ Review
```

## 3.2 Standard Mode

Use for normal features and non-trivial bugs.

Workflow:

```text
Requirements
→ Inspect
→ Design
→ Acceptance Criteria
→ Tasks
→ Implement
→ Test
→ Validate
→ Review
→ Document
```

## 3.3 Full Mode

Use for large, cross-cutting, architectural, or high-risk work.

Workflow:

```text
PRD
→ Research
→ Architecture
→ Technical Design
→ Security Review
→ Acceptance Criteria
→ Task Breakdown
→ Incremental Implementation
→ Test Strategy
→ Validation
→ Review
→ ADRs
→ Documentation Sync
```

Do not force full-mode ceremony onto trivial tasks.

---

# 14. Change / Risk Classification

## 14.1 Low Risk

Examples:

- text/documentation changes,
- isolated UI changes,
- small refactors,
- straightforward tests.

Use a lightweight process.

## 14.2 Medium Risk

Examples:

- new API endpoint,
- new database table,
- authentication changes,
- external service integration.

Use requirements + design + tasks + tests.

## 14.3 High Risk

Examples:

- payments,
- authorization/security boundaries,
- destructive database changes,
- production migrations,
- concurrency-sensitive systems,
- critical infrastructure,
- large architectural changes.

Use:

- explicit design,
- failure analysis,
- rollback considerations,
- stronger testing,
- careful review,
- documentation of decisions.

---
