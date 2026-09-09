# 19. Error Handling

For each important operation, consider:

```text
Input invalid
Permission denied
Resource missing
Conflict
Timeout
Network failure
External service failure
Database failure
Unexpected internal error
```

Errors should be:

- predictable,
- observable,
- safe,
- actionable.

Do not silently swallow exceptions unless the behavior is intentional and documented.

Do not add elaborate handling for impossible scenarios merely for completeness.

---
