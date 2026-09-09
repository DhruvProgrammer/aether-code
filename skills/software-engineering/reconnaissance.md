# 6. Repository Reconnaissance

For an existing repository, first establish a mental model.

Inspect:

- repository tree,
- application entry points,
- package/dependency configuration,
- configuration/environment handling,
- core modules,
- data layer,
- APIs,
- tests,
- deployment configuration,
- project instruction files.

A useful model is:

```text
Repository
├── Entry Points
├── Core Modules
├── Infrastructure
├── Data Layer
├── APIs
├── Tests
├── Configuration
└── Deployment
```

For the specific requested feature, trace the relevant execution path.

Do not assume code exists where it has not been inspected.

---
