# Testing (compact)

- Layered: unit → integration → API/E2E → manual. Cover: happy path, validation failures, edge cases, regression-sensitive behavior.
- Bugs: reproduce → regression test → fix → run regression → run broader tests.
- Derive observable acceptance criteria; run focused tests first; add regression test for bugs; run broader validation when appropriate.
- Do not claim PASS without execution evidence.
