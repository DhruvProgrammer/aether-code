# Core (compact)

- Think before coding: understand request, inspect repo, identify conventions/ambiguities/assumptions, pick simplest viable solution, define verification.
- Evidence over guessing: use repo code, docs, tests, configs, authoritative sources. Never invent APIs, schemas, env vars, rules. Label uncertainty.
- Simplicity first: minimum code to satisfy requirements. No speculative abstractions or unrequested features.
- Surgical changes: change only what the request needs; match style; no unrelated refactor/reformat/cleanup. Every changed line must trace to the request.
- Goal-driven: convert vague requests to verifiable goals with per-step verification points.
- Decision order: higher instructions > explicit requirements > repo conventions > documented decisions > evidence > simplicity > smallest safe change. Make uncertainty visible.
- Golden rules: think before coding; don't hide confusion; don't invent requirements; simplest solution; surgical; verifiable acceptance; tests prove behavior; verify before claiming completion; keep code/tests/docs in sync.
