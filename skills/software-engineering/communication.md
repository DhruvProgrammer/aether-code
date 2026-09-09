# 28. AI Communication Protocol

When reporting progress, distinguish:

### Confirmed
Facts verified from repository evidence, tests, or authoritative sources.

### Assumed
A choice made because a requirement was not explicit.

### Unknown
Something not yet established.

### Blocked
Something that genuinely prevents safe progress.

### Completed
A task whose acceptance criteria and required validation passed.

Do not claim:

- tests passed when they were not run;
- a bug is fixed when it was not reproduced/verified where practical;
- an API works when its actual behavior was not checked;
- documentation is updated when it was not.

---

# 29. Completion Report

At the end of meaningful work, report:

```markdown
## Completed

- TASK-001 — ...
- TASK-002 — ...

## Validation

- Tests: ...
- Lint: ...
- Type check: ...
- Build: ...

## Files Changed

- ...

## Assumptions

- ...

## Known Limitations

- ...

## Documentation Updated

- ...
```

Keep the report factual and concise.

---

# 30. Final Review Checklist

Before declaring completion, verify:

### Requirements
- [ ] Every in-scope requirement is addressed.
- [ ] No major requirement was silently reinterpreted.
- [ ] Assumptions are identified.

### Simplicity
- [ ] No speculative feature was added.
- [ ] No unnecessary abstraction was introduced.
- [ ] No unnecessary dependency was added.
- [ ] The implementation is as simple as practical.

### Surgical Changes
- [ ] Unrelated code was not modified.
- [ ] Existing conventions were followed.
- [ ] New orphaned imports/variables/functions were removed.
- [ ] Pre-existing unrelated dead code was left alone unless requested.

### Correctness
- [ ] Acceptance criteria were checked.
- [ ] Relevant tests were run.
- [ ] Relevant validation commands were run.
- [ ] Important edge cases were considered.

### Safety
- [ ] Security implications were considered.
- [ ] Secrets are not exposed.
- [ ] Compatibility was considered.
- [ ] Destructive operations were reviewed.

### Documentation
- [ ] Relevant docs reflect the actual system.
- [ ] API/schema/configuration documentation was updated when needed.
- [ ] Important architectural decisions are recorded.

### Diff
- [ ] Final diff was reviewed.
- [ ] No accidental files or unrelated edits remain.

---
