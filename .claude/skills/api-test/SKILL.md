---
name: api-test
description: "HISTORICAL redirect — the in-process analytics HTTP contract rig this skill documented was retired; do not use this skill to write tests. Analytics HTTP contract coverage lives in tests/stand/api/ (use stand-api-test); seeded-bronze → served-metric specs live in tests/datapath/metrics/<class>/ (use metric-test). This stub exists only so old references resolve."
disable-model-invocation: true
user-invocable: true
allowed-tools: Read, Grep, Glob
---

# api-test — retired

The suite this skill documented — an in-process analytics endpoint contract
rig — no longer exists; its HTTP contract lanes were retired in favour of the
deployed-stand suite.

Where the work goes now:

- **HTTP contract tests** → `tests/stand/api/`, written per `stand-api-test`.
- **Data-path specs** (seeded bronze → served metric, run against a compose
  test-stand instance) → `tests/datapath/metrics/<class>/`, written per
  `metric-test`.

The retired rig's documentation and code live in git history:
`git log --all -- src/ingestion/tests/e2e/api`.
