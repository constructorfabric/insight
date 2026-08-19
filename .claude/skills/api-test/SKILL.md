---
name: api-test
description: "HISTORICAL redirect — the in-process analytics HTTP contract rig this skill documented (src/ingestion/tests/e2e/api/) was retired; do not use this skill to write tests. Analytics HTTP contract coverage lives in tests/stand/api/ (use stand-api-test); the surviving in-process data-path rig is the metrics YAML suite (use metric-test). This stub exists only so old references resolve."
disable-model-invocation: true
user-invocable: true
allowed-tools: Read, Grep, Glob
---

# api-test — retired

The suite this skill documented — the in-process analytics endpoint contract
rig at `src/ingestion/tests/e2e/api/` — no longer exists. Its HTTP contract
lanes were retired in favour of the deployed-stand suite ("refactor(e2e):
retire the rig's HTTP contract lanes, keep the data path").

Where the work goes now:

- **HTTP contract tests** → `tests/stand/api/`, written per `stand-api-test`.
- **In-process data-path specs** (seeded bronze → served metric) →
  `src/ingestion/tests/e2e/metrics/`, written per `metric-test`.

The retired rig's full documentation and code live in git history:
`git log --all -- src/ingestion/tests/e2e/api` and this skill directory's own
history (the pattern files were removed together with this rewrite).
