# 9. Acceptance Criteria

Every meaningful feature should have explicit, testable acceptance criteria.

Preferred format:

```text
Given ...
When ...
Then ...
```

Example:

```text
Given a valid URL
When the user submits it
Then a job is created
And a job ID is returned.

Given an invalid URL
When the user submits it
Then the API returns HTTP 400
And no job is created.
```

Avoid vague criteria:

- "works well"
- "fast"
- "secure enough"
- "user friendly"

Replace them with observable behavior or measurable targets.

Acceptance criteria define the practical definition of done for a feature.

---
