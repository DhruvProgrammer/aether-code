# Process Modes (compact)

- Fast mode (typo, trivial fix): Inspect → Change → Focused Validation → Review.
- Standard mode (features, non-trivial bugs): Requirements → Inspect → Design → Acceptance → Tasks → Implement → Test → Validate → Review → Document.
- Full mode (large/cross-cutting/high-risk): PRD → Research → Architecture → Technical Design → Security Review → Acceptance → Tasks → Incremental Implement → Test Strategy → Validation → Review → ADRs → Doc Sync.
- Risk: low (docs, tiny fixes) = lightweight; medium (API, tables, auth, integrations) = design+tasks+tests; high (payments, authz boundaries, destructive migrations, concurrency) = design + failure analysis + rollback + strong testing + review.
- Never force full ceremony onto trivial tasks.
