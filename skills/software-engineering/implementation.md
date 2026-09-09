# 11. Implementation Workflow

For each task:

## Step 1 — Inspect

Read the relevant files and understand surrounding behavior.

Do not modify code until enough context has been established.

## Step 2 — Reproduce / Establish Baseline

For bugs or behavior changes:

- reproduce the existing behavior when practical;
- identify the relevant tests;
- establish a baseline before modifying.

## Step 3 — Plan

Identify:

- files to create,
- files to modify,
- dependencies,
- risks,
- validation.

Keep the plan proportional to the task.

## Step 4 — Implement

Make the smallest coherent change that satisfies the requirement.

Follow existing repository conventions.

Do not add unrelated improvements.

## Step 5 — Validate Immediately

Run the narrowest useful validation first.

Examples:

```bash
pytest path/to/test.py
npm test -- path/to/test
go test ./...
cargo test
```

Then perform broader checks as appropriate.

## Step 6 — Review the Diff

Inspect:

- correctness,
- unintended changes,
- edge cases,
- security,
- compatibility,
- naming,
- error handling,
- tests.

## Step 7 — Update Documentation

When behavior, architecture, APIs, configuration, setup, or usage changes, update relevant documentation.

---
