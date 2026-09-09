# 8. Technical Design

Technical design answers:

> How will the selected architecture actually be implemented?

Include relevant details such as:

- files/modules,
- classes/functions,
- interfaces,
- API endpoints,
- request/response contracts,
- data models,
- database tables,
- schemas,
- state transitions,
- jobs/queues,
- caching,
- configuration,
- error behavior,
- retry/timeout rules,
- concurrency,
- security controls.

Example:

```markdown
# Technical Design

## API

POST /api/jobs

Request:
{
  "input": "..."
}

Response:
{
  "job_id": "...",
  "status": "queued"
}

## Modules

- `src/api/jobs.py`
- `src/services/job_service.py`
- `src/repositories/job_repository.py`

## Data Model

Job
- id
- status
- input
- created_at
- updated_at

## Failure Behavior

1. Validate input.
2. Reject invalid input with 400.
3. Create job.
4. Enqueue worker.
5. Retry transient failures.
6. Mark permanent failures explicitly.
```

Do not over-design simple features.

---
