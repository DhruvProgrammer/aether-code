# Memory Evaluation — acceptance framework (task-spec §47–49, §63)

## 1. Buried-information test (§49, the most important test)

`engine::tests::buried_information_is_retrievable`: seed ~120 filler
records across episodes, bury one decision + one correction deep in history,
then retrieve with a task request that shares vocabulary but not exact
phrases. Assert: target in top results, tokens packed < full-history
tokens, no superseded record ranked above its replacement.

## 2. Scenario matrix (§47)

| Scenario | What must be retrieved |
|---|---|
| recent task | latest episode + current files |
| old requirement | original requirement record, not its superseded draft |
| exact error string | error + fix + verification trio |
| file-name query | file_insight + implementation touching that path |
| contradiction | newer correction only; older fact penalized |
| multi-hop | decision via episode → file → implementation chain |
| stale workspace | record whose file no longer matches is flagged stale |

Each scenario asserts precision (target present), stale rate (no
superseded record above its replacement), and budget (packed tokens <=
budget).

## 3. Long-session benchmark (§48)

Scripted 100-turn synthetic session (define → investigate → decide →
implement → bug → fix → requirement change → verify → distract → resume).
At the final turn assert the packed context contains: objective, latest
requirements, architecture decision, implementation state, files, bug
history, verification, next action — while packed tokens stay under 15% of
raw transcript tokens.

## 4. Metrics recorded per retrieval

precision, recall@k, stale-retrieval rate, contradiction rate, packed
tokens vs budget, latency. Diagnostics (`RetrievalDiagnostics`) make every
number reproducible.
