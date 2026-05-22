---
status: proposed
date: 2026-05-21
---

# Feature: CSV-driven Test Rig (vertical slice MVP)

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-featstatus-csv-rig`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Author and Run a Test](#author-and-run-a-test)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Execute Test (per-test loop)](#execute-test-per-test-loop)
  - [Truncate Touched Bronze Tables](#truncate-touched-bronze-tables)
- [4. States (CDSL)](#4-states-cdsl)
  - [Test Lifecycle](#test-lifecycle)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Folder Discovery and Fixture Loading](#folder-discovery-and-fixture-loading)
  - [Typed Bronze INSERT](#typed-bronze-insert)
  - [Scoped Dbt Build](#scoped-dbt-build)
  - [API Roundtrip](#api-roundtrip)
  - [Cell-Precise CSV Diff](#cell-precise-csv-diff)
  - [Reference Test against insight.people](#reference-test-against-insightpeople)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

## 1. Feature Context

- [ ] `p2` - `cpt-bronze-to-api-e2e-feature-csv-rig`

### 1.1 Overview

Cut a single working end-to-end test path through every layer of the Bronze-to-API E2E Test Framework. After this feature lands, a developer can author a fixture folder (`bronze/*.csv` + `spec.yaml` + `expected/response.csv`) and watch the runner load it, INSERT typed rows into bronze, run a scoped `dbt build`, call the analytics-api on a loopback port, and diff the response against the expected CSV — with cell-precise output on failure.

This feature is the integration of every component from DESIGN (`fixture-loader`, `ch-seeder`, `dbt-runner`, `api-client`, `csv-asserter`, `session-rig`) plus one reference fixture (`fixtures/people_smoke/`) that exercises `insight.people` end-to-end. The reference fixture is part of the DoD — without it, the framework has no proof of life.

### 1.2 Purpose

The framework as a whole exists to give data engineers and backend developers a fast same-day signal that a dbt model, migration view, or analytics-api code change is consistent with the contract the UI consumes (`cpt-bronze-to-api-e2e-fr-bronze-seed-from-csv`, `cpt-bronze-to-api-e2e-fr-csv-assert`, `cpt-bronze-to-api-e2e-fr-api-roundtrip`). This feature is the MVP that delivers that signal for one view and one metric — the smallest scope that lets the framework be evaluated end-to-end.

**Requirements**:

- `cpt-bronze-to-api-e2e-fr-bronze-seed-from-csv`
- `cpt-bronze-to-api-e2e-fr-bronze-truncate`
- `cpt-bronze-to-api-e2e-fr-dbt-run-scoped`
- `cpt-bronze-to-api-e2e-fr-gold-view-queried`
- `cpt-bronze-to-api-e2e-fr-api-roundtrip`
- `cpt-bronze-to-api-e2e-fr-csv-assert`
- `cpt-bronze-to-api-e2e-nfr-per-test-latency`

**Principles**:

- `cpt-bronze-to-api-e2e-principle-shared-session`
- `cpt-bronze-to-api-e2e-principle-fixtures-are-truth`

**Constraints**:

- `cpt-bronze-to-api-e2e-constraint-no-ddl-mutation`
- `cpt-bronze-to-api-e2e-constraint-loopback-only`

### 1.3 Actors

| Actor | Role in Feature |
|-------|-----------------|
| `cpt-bronze-to-api-e2e-actor-test-author` | Authors the fixture folder and runs `pytest` |
| `cpt-bronze-to-api-e2e-actor-data-engineer` | The primary persona for the reference fixture (changes a dbt model and reruns) |
| `cpt-bronze-to-api-e2e-actor-dbt-cli` | Subprocess invoked per test with a selector |
| `cpt-bronze-to-api-e2e-actor-analytics-api` | Service under test, spawned once per session |

### 1.4 References

- **PRD**: [../PRD.md](../PRD.md)
- **DESIGN**: [../DESIGN.md](../DESIGN.md)
- **DECOMPOSITION**: [../DECOMPOSITION.md](../DECOMPOSITION.md)
- **Depends on (features)**:
  - `cpt-bronze-to-api-e2e-feature-test-rig-scaffolding`
  - `cpt-bronze-to-api-e2e-feature-fixture-loader`
  - `cpt-bronze-to-api-e2e-feature-dbt-runner`
  - `cpt-bronze-to-api-e2e-feature-api-spawner`
  - `cpt-bronze-to-api-e2e-feature-csv-asserter`

## 2. Actor Flows (CDSL)

### Author and Run a Test

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-flow-csv-rig-author-and-run`

**Actor**: `cpt-bronze-to-api-e2e-actor-test-author`

**Success Scenarios**:

- Author writes a new fixture targeting `insight.people`, generates `expected/response.csv` via `--update-snapshots`, runs the suite, and the test passes
- Author runs the existing reference fixture (`people_smoke`) after a dbt model change — the diff highlights the regression

**Error Scenarios**:

- `spec.yaml` is malformed or missing required keys → loader fails at session-collect time with the specific JSON-Schema violation
- `expected/response.csv` references a `key_column` that the response items don't contain → loader fails at session-collect time
- The selected dbt model errors out → `dbt-runner` parses run-results and surfaces the failing model's compiled SQL excerpt in the pytest report

**Steps**:

1. [ ] - `p1` - Author creates `src/ingestion/tests/e2e/fixtures/<name>/` with `bronze/<schema>.<table>.csv` files - `inst-author-fixture-dir`
2. [ ] - `p1` - Author writes `spec.yaml` with `endpoint`, `method`, `metric_id`, `request_body`, `dbt_selector`, `key_columns`, optional `float_tolerance` - `inst-author-spec-yaml`
3. [ ] - `p1` - Author runs `pytest --update-snapshots -k <name>` (requires `feature-snapshot-update` to be live) to bootstrap `expected/response.csv` - `inst-author-bootstrap-expected`
4. [ ] - `p1` - **IF** the bootstrap path is not available yet, author writes `expected/response.csv` by hand (small/known scenarios) - `inst-author-hand-write-expected`
5. [ ] - `p1` - Author runs `pytest -k <name>` - `inst-author-run-pytest`
6. [ ] - `p1` - Algorithm: framework runs `cpt-bronze-to-api-e2e-algo-csv-rig-execute-test` - `inst-author-invoke-execute-test`
7. [ ] - `p1` - **IF** test passes **RETURN** "ok" - `inst-author-return-pass`
8. [ ] - `p1` - **ELSE** runner emits cell-precise diff `(key, column, expected, actual)` to pytest captured stdout - `inst-author-emit-diff`
9. [ ] - `p1` - **RETURN** failure signal to pytest - `inst-author-return-fail`

## 3. Processes / Business Logic (CDSL)

### Execute Test (per-test loop)

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-algo-csv-rig-execute-test`

**Input**: `Fixture` (loaded from disk), `WorkerContext` (`worker_id`, `schema_suffix`)

**Output**: `AssertionResult` (Pass | Fail with diff rows)

**Steps**:

1. [ ] - `p1` - Algorithm: invoke `cpt-bronze-to-api-e2e-algo-csv-rig-truncate-touched` to clear last test's bronze rows - `inst-exec-truncate`
2. [ ] - `p1` - **FOR EACH** `bronze/<schema>.<table>.csv` in `Fixture.bronze_csvs` - `inst-exec-foreach-csv`
   1. [ ] - `p1` - DB: SELECT system.columns WHERE database = "<schema><suffix>" AND table = "<table>" — fetch column types - `inst-exec-fetch-types`
   2. [ ] - `p1` - Coerce each CSV cell: empty → SQL NULL; date strings → DateTime; numeric → typed; arrays → ClickHouse array literal - `inst-exec-coerce-cells`
   3. [ ] - `p1` - DB: INSERT INTO `<schema><suffix>`.`<table>` FORMAT CSVWithNames (batched) - `inst-exec-insert-batch`
   4. [ ] - `p1` - Record `(schema, table)` in per-test touched-tables ledger - `inst-exec-ledger-record`
3. [ ] - `p1` - Subprocess: `dbt build --select <Fixture.spec.dbt_selector> --defer --state target/ --vars '{worker_id: <N>}'` - `inst-exec-dbt-build`
4. [ ] - `p1` - **IF** dbt exit code != 0 → parse `target/run_results.json`, surface failing model + compiled SQL excerpt, **RETURN** Fail("dbt build failed") - `inst-exec-dbt-fail`
5. [ ] - `p1` - HTTP: `api-client.call(Fixture.spec.endpoint, Fixture.spec.method, Fixture.spec.request_body)` → `ApiResponse` - `inst-exec-api-call`
6. [ ] - `p1` - **IF** HTTP status != 200 **RETURN** Fail("API status=<N>: <body>") - `inst-exec-api-status-fail`
7. [ ] - `p1` - Algorithm: flatten `ApiResponse.items` into pandas DataFrame `actual_df` - `inst-exec-flatten-items`
8. [ ] - `p1` - **IF** `set(actual_df.columns) != set(Fixture.expected_df.columns)` **RETURN** Fail("column set mismatch: missing=<...> extra=<...>") - `inst-exec-column-mismatch`
9. [ ] - `p1` - Sort both DataFrames by `Fixture.spec.key_columns`; align column order - `inst-exec-sort-align`
10. [ ] - `p1` - **TRY** `assert_frame_equal(actual_df, Fixture.expected_df, atol=Fixture.spec.float_tolerance)` - `inst-exec-assert-equal`
11. [ ] - `p1` - **CATCH** AssertionError → diff into `(key, column, expected, actual)` rows; take first 20; **RETURN** Fail(diff_rows) - `inst-exec-catch-diff`
12. [ ] - `p1` - **RETURN** Pass - `inst-exec-return-pass`

### Truncate Touched Bronze Tables

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-algo-csv-rig-truncate-touched`

**Input**: per-test touched-tables ledger from the previous test (may be empty for the first test)

**Output**: void (side effect: ClickHouse bronze tables are empty for the touched set)

**Steps**:

1. [ ] - `p1` - **IF** ledger is empty **RETURN** void - `inst-trunc-empty-return`
2. [ ] - `p1` - **FOR EACH** `(schema, table)` in ledger - `inst-trunc-foreach`
   1. [ ] - `p1` - DB: TRUNCATE TABLE `<schema><suffix>`.`<table>` - `inst-trunc-truncate`
3. [ ] - `p1` - Clear ledger - `inst-trunc-clear-ledger`
4. [ ] - `p1` - **RETURN** void - `inst-trunc-return`

## 4. States (CDSL)

### Test Lifecycle

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-state-csv-rig-lifecycle`

**States**: `PENDING`, `SEEDING`, `DBT_BUILDING`, `API_QUERYING`, `ASSERTING`, `PASSED`, `FAILED`

**Initial State**: `PENDING`

**Transitions**:

1. [ ] - `p1` - **FROM** `PENDING` **TO** `SEEDING` **WHEN** per-test fixture entered and ledger truncated - `inst-state-pending-to-seeding`
2. [ ] - `p1` - **FROM** `SEEDING` **TO** `DBT_BUILDING` **WHEN** all `bronze/*.csv` files inserted without error - `inst-state-seeding-to-dbt`
3. [ ] - `p1` - **FROM** `SEEDING` **TO** `FAILED` **WHEN** any CSV INSERT raises (e.g., type coercion error, ClickHouse error) - `inst-state-seeding-to-failed`
4. [ ] - `p1` - **FROM** `DBT_BUILDING` **TO** `API_QUERYING` **WHEN** `dbt build` exits with code 0 - `inst-state-dbt-to-api`
5. [ ] - `p1` - **FROM** `DBT_BUILDING` **TO** `FAILED` **WHEN** `dbt build` exits non-zero - `inst-state-dbt-to-failed`
6. [ ] - `p1` - **FROM** `API_QUERYING` **TO** `ASSERTING` **WHEN** HTTP status 200 received and body deserialized - `inst-state-api-to-asserting`
7. [ ] - `p1` - **FROM** `API_QUERYING` **TO** `FAILED` **WHEN** HTTP status ≠ 200 or body cannot be deserialized - `inst-state-api-to-failed`
8. [ ] - `p1` - **FROM** `ASSERTING` **TO** `PASSED` **WHEN** `assert_frame_equal` does not raise - `inst-state-asserting-to-passed`
9. [ ] - `p1` - **FROM** `ASSERTING` **TO** `FAILED` **WHEN** `assert_frame_equal` raises; diff rows captured - `inst-state-asserting-to-failed`

## 5. Definitions of Done

### Folder Discovery and Fixture Loading

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-dod-csv-rig-folder-discovery`

The system **MUST** discover every subfolder of `src/ingestion/tests/e2e/fixtures/` as a candidate fixture and load each into a `Fixture` value at session-collect time. A misshaped `spec.yaml` or a missing `expected/response.csv` **MUST** fail the collection of that fixture (not the entire session) with a specific error.

**Implements**:

- `cpt-bronze-to-api-e2e-flow-csv-rig-author-and-run`

**Constraints**: `cpt-bronze-to-api-e2e-constraint-no-ddl-mutation`

**Touches**:

- Files: `src/ingestion/tests/e2e/e2e_lib/fixture_loader.py`, `src/ingestion/tests/e2e/conftest.py`
- Components: `cpt-bronze-to-api-e2e-component-fixture-loader`

### Typed Bronze INSERT

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-dod-csv-rig-typed-bronze-insert`

The system **MUST** INSERT each CSV row into the bronze table with column types resolved via `system.columns`. Empty cells **MUST** become SQL NULL. Date-like and array-like values **MUST** be coerced to their ClickHouse counterparts. At least three row types — string, integer, timestamp — **MUST** be exercised by the reference test.

**Implements**:

- `cpt-bronze-to-api-e2e-algo-csv-rig-execute-test`

**Touches**:

- API: ClickHouse HTTP 8123 / native 9000
- Files: `src/ingestion/tests/e2e/e2e_lib/ch_seeder.py`
- Components: `cpt-bronze-to-api-e2e-component-ch-seeder`

### Scoped Dbt Build

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-dod-csv-rig-scoped-dbt`

The system **MUST** invoke `dbt build` with the selector from `spec.yaml` (e.g. `+silver_people+`) and **MUST NOT** run any model outside that selector's transitive closure. The session-cached `target/manifest.json` and `--defer --state target/` flags **MUST** be used.

**Implements**:

- `cpt-bronze-to-api-e2e-algo-csv-rig-execute-test`

**Constraints**: `cpt-bronze-to-api-e2e-constraint-no-ddl-mutation`

**Touches**:

- Files: `src/ingestion/tests/e2e/e2e_lib/dbt_runner.py`
- Components: `cpt-bronze-to-api-e2e-component-dbt-runner`

### API Roundtrip

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-dod-csv-rig-api-roundtrip`

The system **MUST** POST the request derived from `spec.yaml` to the analytics-api over HTTP loopback and deserialize the JSON body into a typed `ApiResponse`. Auth **MUST** be disabled in the spawned binary. The api-client **MUST** be the same instance for every test in the session.

**Implements**:

- `cpt-bronze-to-api-e2e-algo-csv-rig-execute-test`

**Constraints**: `cpt-bronze-to-api-e2e-constraint-loopback-only`

**Touches**:

- API: `POST /v1/metrics/{id}/query`
- Files: `src/ingestion/tests/e2e/e2e_lib/api_client.py`
- Components: `cpt-bronze-to-api-e2e-component-api-client`

### Cell-Precise CSV Diff

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-dod-csv-rig-cell-precise-diff`

On failure the system **MUST** render the first 20 mismatched cells as `(key, column, expected, actual)` lines, write them to the pytest captured stdout (not just a log file), and raise an `AssertionError` with the same text. Column-set mismatches and row-count mismatches **MUST** also render cell-shaped diff lines (with placeholder values for missing rows).

**Implements**:

- `cpt-bronze-to-api-e2e-algo-csv-rig-execute-test`

**Touches**:

- Files: `src/ingestion/tests/e2e/e2e_lib/csv_asserter.py`
- Components: `cpt-bronze-to-api-e2e-component-csv-asserter`

### Reference Test against insight.people

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-dod-csv-rig-people-reference-test`

The system **MUST** ship one fully working fixture at `src/ingestion/tests/e2e/fixtures/people_smoke/` that exercises `insight.people` end-to-end. The fixture **MUST** include bronze inputs for at least one connector (e.g. `bronze_bamboohr.employees`), a `spec.yaml` calling `POST /v1/metrics/{id}/query` against a seeded metric, and an `expected/response.csv` that passes against an unmodified main-branch checkout. The reference test **MUST** complete within 5 s on a warm session.

**Implements**:

- `cpt-bronze-to-api-e2e-flow-csv-rig-author-and-run`

**Touches**:

- Files: `src/ingestion/tests/e2e/fixtures/people_smoke/{bronze/*.csv,spec.yaml,expected/response.csv}`
- Components: integrates all components above

## 6. Acceptance Criteria

- [ ] **Given** an empty `tests/e2e/fixtures/` and a fresh checkout, **When** a developer adds `fixtures/people_smoke/` with valid contents and runs `pytest src/ingestion/tests/e2e/`, **Then** the test passes within 5 s after the session has warmed up
- [ ] **Given** the reference fixture passes against main, **When** a developer breaks a relevant dbt model (e.g. renames a column in `silver_people`) and reruns the suite, **Then** the test fails and the pytest captured stdout includes ≥ 1 cell-precise diff line of the form `(row_key=…, column=…, expected=…, actual=…)`
- [ ] **Given** the framework is running, **When** a developer authors a malformed `spec.yaml` (missing `key_columns`), **Then** pytest reports the JSON-Schema violation at session-collect time and does not attempt to run that test
- [ ] **Given** the reference fixture passes once, **When** the suite is rerun in the same session, **Then** the second run completes in ≤ 5 s (proving TRUNCATE-not-DROP isolation is effective)
- [ ] **Given** the framework is running with `pytest -n 2`, **When** two workers run the reference fixture concurrently, **Then** both pass and neither sees the other's bronze data (verified by inspecting touched-tables ledgers in test output)
