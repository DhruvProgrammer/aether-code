# Error Handling (compact)

- Consider per operation: invalid input, denied permission, missing resource, conflict, timeout, network/external/db failure, internal error.
- Errors: predictable, observable, safe, actionable. Don't swallow exceptions unless intentional and documented. No elaborate handling for impossible scenarios.
