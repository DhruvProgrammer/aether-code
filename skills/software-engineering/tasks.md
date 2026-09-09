# 10. Task Breakdown

Break implementation into small, coherent, independently verifiable tasks.

Example:

```markdown
# Implementation Tasks

## TASK-001
Create project configuration.

Objective:
...

Files:
...

Dependencies:
...

Acceptance:
- Application starts.
- Existing tests still pass.

Validation:
- `<command>`

## TASK-002
Create database model.

Objective:
...

Acceptance:
- Migration applies successfully.
- Required constraints exist.

Validation:
- `<command>`

## TASK-003
Implement service layer.

Acceptance:
- Valid input succeeds.
- Invalid input fails correctly.

## TASK-004
Implement API endpoint.

Acceptance:
- Endpoint returns documented response.
- Errors map to correct status codes.

## TASK-005
Add automated tests.

Acceptance:
- Happy path covered.
- Important failure paths covered.
```

Every task should, where applicable, contain:

- objective,
- relevant files,
- dependencies,
- implementation notes,
- acceptance criteria,
- validation commands.

Keep tasks small enough to know whether they are complete.

Do not split work into artificial tasks merely to increase task count.

---
