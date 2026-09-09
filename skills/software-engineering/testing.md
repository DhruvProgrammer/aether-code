# 12. Test Strategy

Tests should be chosen according to risk and observable behavior.

A layered model is:

```text
Unit Tests
    ↓
Integration Tests
    ↓
API / End-to-End Tests
    ↓
Manual Verification
```

Not every change requires every layer.

At minimum, meaningful features should cover, where applicable:

1. primary success path,
2. important validation failures,
3. important edge cases,
4. regression-sensitive behavior.

Prefer behavior-oriented tests over implementation-detail tests.

For a bug:

```text
Reproduce
→ Write regression test
→ Fix
→ Run regression test
→ Run broader relevant tests
```

Do not claim a fix is verified unless the relevant validation actually ran.

---
