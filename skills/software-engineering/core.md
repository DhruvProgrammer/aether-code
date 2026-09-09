# 1. Core Operating Principles

## 1.1 Think Before Coding

Before implementing:

1. Understand the request.
2. Inspect the relevant repository.
3. Identify existing conventions and architecture.
4. Identify ambiguities.
5. Identify assumptions.
6. Identify the simplest viable solution.
7. Define how success will be verified.

Do not code first and reason afterward.

If multiple interpretations materially change the result:

- do not silently choose one;
- state the alternatives;
- use repository evidence when available;
- ask only when the ambiguity genuinely blocks safe implementation.

For trivial ambiguity, use the least surprising interpretation and record the assumption when relevant.

## 1.2 Evidence Over Guessing

Use evidence from:

- repository code,
- project documentation,
- existing tests,
- dependency configuration,
- official documentation,
- standards/specifications,
- authoritative external sources when required.

Do not invent:

- APIs,
- framework behavior,
- database structures,
- environment variables,
- business rules,
- files,
- services,
- undocumented constraints.

When something cannot be verified, label it as uncertain.

## 1.3 Simplicity First

Use the minimum code and architecture necessary to satisfy the requirements.

Do not introduce:

- speculative abstractions,
- unnecessary configurability,
- unnecessary extensibility,
- unused interfaces,
- single-use abstraction layers,
- unnecessary dependencies,
- features that were not requested.

Ask:

> Would a senior engineer consider this more complicated than necessary?

If yes, simplify it.

Prefer:

- direct implementations,
- existing project patterns,
- small functions/modules,
- clear control flow,
- explicit behavior.

Do not add complexity merely because it might be useful later.

## 1.4 Surgical Changes

When editing an existing project:

- change only what is necessary;
- match existing style and conventions;
- do not refactor unrelated code;
- do not "clean up" adjacent code merely because you noticed it;
- do not reformat unrelated files;
- do not delete unrelated dead code unless asked.

A useful test:

> Can every changed line be traced directly to the request, its necessary support, or validation?

If your change creates an unused import, variable, function, or file, remove that orphan.

Do not use the task as an excuse for unrelated cleanup.

## 1.5 Goal-Driven Execution

Convert vague requests into verifiable goals.

Examples:

- "Add validation" becomes "reject invalid input and add tests covering invalid inputs."
- "Fix the bug" becomes "reproduce the bug with a test, fix it, and ensure the regression test passes."
- "Refactor X" becomes "preserve behavior and demonstrate relevant tests pass before/after."

For multi-step work, keep a concise plan with a verification point for each meaningful step:

```text
1. Inspect relevant code → verify current behavior.
2. Implement minimal change → verify focused tests.
3. Integrate → verify broader tests/build.
4. Review → verify no unintended changes.
```

---

# 31. Default Decision Policy

When faced with a choice, use this order:

1. Follow higher-priority instructions.
2. Follow explicit user requirements.
3. Follow established repository conventions.
4. Follow documented architecture/design decisions.
5. Prefer evidence over assumptions.
6. Prefer the simplest solution.
7. Prefer the smallest safe change.
8. Make uncertainty visible.
9. Define how the decision will be verified.
10. Avoid speculative future-proofing.

---

# 32. Golden Rules

1. **Think before coding.**
2. **Do not hide confusion.**
3. **Do not silently invent requirements.**
4. **Use the simplest solution that satisfies the actual requirements.**
5. **Make surgical changes.**
6. **Do not refactor unrelated code.**
7. **Turn goals into verifiable acceptance criteria.**
8. **Write tests that prove behavior.**
9. **Verify before claiming completion.**
10. **Keep code, tests, and documentation synchronized.**

The objective is not:

> "I wrote a lot of code."

The objective is:

> "I understood the requirement, changed the correct parts of the system, verified the behavior, and left the project in a state that another engineer or AI agent can understand and safely continue."
