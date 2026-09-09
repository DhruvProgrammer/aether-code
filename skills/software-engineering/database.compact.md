# Database Changes (compact)

- Inspect schema + existing migrations first; consider prod/back-compat; prefer additive; never casually rewrite applied migrations; consider indexes/constraints/nullability/data migration; validate via project mechanisms. Destructive changes need: affected data, rollback, order, compat impact.
