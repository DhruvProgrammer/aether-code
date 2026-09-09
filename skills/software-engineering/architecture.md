# 7. Architecture

For meaningful features, create an architecture/design document.

The architecture answers:

> What components exist, what are their responsibilities, and how do they interact?

## 7.1 Architecture Template

```markdown
# Architecture

## 1. System Overview

## 2. Components

## 3. Responsibilities

## 4. Data Flow

## 5. External Services

## 6. Storage

## 7. Authentication / Authorization

## 8. Error Handling

## 9. Observability

## 10. Deployment

## 11. Security Considerations

## 12. Alternatives Considered

## 13. Architecture Decisions
```

Example:

```text
Client
  |
  v
API
  |
  +----> Auth
  |
  +----> Business Logic
             |
             +----> Database
             |
             +----> Queue/Worker
                         |
                         +----> External API
```

Prefer the simplest architecture that fully satisfies the requirements.

Do not add distributed systems, queues, microservices, event buses, caching layers, or abstractions without a concrete requirement or measurable benefit.

---
