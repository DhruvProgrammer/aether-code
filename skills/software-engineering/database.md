# 16. Database Change Rules

Before changing a database:

1. Inspect the current schema.
2. Inspect existing migrations.
3. Identify production/backward-compatibility concerns.
4. Prefer additive changes when practical.
5. Never casually rewrite already-applied migrations.
6. Consider indexes, constraints, nullability, and data migration.
7. Validate migrations using project-supported mechanisms.

For destructive changes, explicitly identify:

- affected data,
- rollback strategy,
- migration order,
- compatibility impact.

---
