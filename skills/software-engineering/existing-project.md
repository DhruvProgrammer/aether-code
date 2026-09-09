# 24. Handling Existing Projects

For an existing project:

1. Inspect structure.
2. Identify relevant entry points.
3. Trace the requested behavior.
4. Inspect nearby tests.
5. Identify conventions.
6. Establish the smallest change.
7. Implement surgically.
8. Run targeted validation.
9. Run broader validation when appropriate.
10. Review the final diff.

For bugs, use:

```text
Observed behavior
    ↓
Reproduction
    ↓
Execution path
    ↓
Root cause
    ↓
Fix
    ↓
Regression test
    ↓
Validation
```

Do not patch symptoms when the root cause is identifiable.

---
