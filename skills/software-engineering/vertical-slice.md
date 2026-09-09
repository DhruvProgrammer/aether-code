# 26. Vertical Slice Strategy

For multi-component features, prefer an end-to-end vertical slice when practical:

```text
UI
 ↓
API
 ↓
Business Logic
 ↓
Database
 ↓
External Dependency
 ↓
Test
```

A working slice exposes architecture problems early and gives a usable increment.

Do not build a giant foundation before validating the actual product path unless the requirements truly demand it.

---
