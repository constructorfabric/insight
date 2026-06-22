# PRD — Bronze-to-API E2E Test Framework

<!-- toc -->

- [Changelog](#changelog)
- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals (Business Outcomes)](#13-goals-business-outcomes)
  - [1.4 Glossary](#14-glossary)
- [2. Actors](#2-actors)
  - [2.1 Human Actors](#21-human-actors)
  - [2.2 System Actors](#22-system-actors)
- [3. Operational Concept & Environment](#3-operational-concept--environment)
  - [3.1 Module-Specific Environment Constraints](#31-module-specific-environment-constraints)
- [4. Scope](#4-scope)
  - [4.1 In Scope](#41-in-scope)
  - [4.2 Out of Scope](#42-out-of-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 Bronze Seeding](#51-bronze-seeding)
  - [5.2 dbt Execution](#52-dbt-execution)
  - [5.3 Migration Views](#53-migration-views)
  - [5.4 API Roundtrip](#54-api-roundtrip)
  - [5.5 Assertion](#55-assertion)
  - [5.6 Isolation](#56-isolation)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## Changelog

- **v1.0** (current): Initial PRD. Establishes the Bronze-to-API E2E Test Framework as a developer-facing tool: each test is a folder of CSV fixtures (bronze inputs + expected API response) and the runner exercises the full data path bronze → dbt staging/silver → ClickHouse gold views (from migrations) → analytics-api HTTP response → CSV assert. Airbyte sync, Argo/Kestra orchestration, and the frontend are explicitly out of v1 scope. v1 ships pytest + local docker compose with one shared ClickHouse and one shared MariaDB per session.

## 1. Overview

### 1.1 Purpose

The Bronze-to-API E2E Test Framework is a developer-facing test harness that exercises Constructor Insight's full data transformation path — from a connector-shaped bronze row to an analytics-api HTTP response — using a declarative, folder-based fixture format. Each test is a folder of CSV files describing what the connector would have written to bronze and what the API is expected to return; the runner seeds bronze, runs dbt, applies the migration-defined gold views, calls the API, and diffs the response against the expected CSV.

It exists so that a developer changing any layer of the pipeline (dbt model, migration view, analytics-api SQL builder, OData filter parser) gets a same-day signal that their change is consistent with the contract the UI consumes — without standing up a kind cluster, without running Airbyte, and without waiting for an external system.

### 1.2 Background / Problem Statement

Today the only automated coverage of Insight's transformation pipeline is dbt's own generic tests (`unique`, `not_null`) plus a small set of hand-written assertion tests under `src/ingestion/dbt/tests/`. They catch dbt-level invariants on silver tables but they don't observe the gold views (created by ClickHouse migrations in `src/ingestion/scripts/migrations/`) and they don't observe the analytics-api response shape that the UI actually reads. A typical regression — a renamed column in a dbt model, a changed `argMax` in a migration view, a tightened OData filter parser in the Rust service — sneaks past CI and gets caught either by a developer running the stack locally or by a tenant in production.

The transformation chain has four authoring surfaces:

- **dbt models** under `src/ingestion/silver/` and `src/ingestion/connectors/*/dbt/`
- **ClickHouse migration views** (~28 gold views, e.g. `insight.people`, `insight.commits_daily`) under `src/ingestion/scripts/migrations/`
- **analytics-api Rust code** under `src/backend/services/analytics-api/` (metric query builder, OData filter parser, ClickHouse SQL emission)
- **Metric catalog rows** in MariaDB (`analytics.metrics.query_ref`)

A change to any one of these surfaces can break the contract `Frontend ← analytics-api ← gold view ← silver`. There is currently no test rig that observes the end-to-end contract on the same machine where the developer is editing.

The related "is the data correct?" question (whether a metric value is semantically right for a tenant's situation) is a different problem and is **not** solved by this PRD. This framework checks that the pipeline emits the expected value given a known input; it does not check that the input itself reflects reality.

**Target Users**:

- Data engineers authoring or changing dbt models and migration views
- Backend developers changing analytics-api endpoints, query builder, or filter parsing
- CI pipelines verifying PRs that touch any of the four authoring surfaces above

**Key Problems Solved**:

- No coverage of the gold-view layer (migration-defined views) in current tests
- No coverage of the analytics-api response shape in current tests
- Setting up a full local environment to test a single dbt/view/api change takes minutes; this framework should take seconds per test

### 1.3 Goals (Business Outcomes)

**Success Criteria**:

- ≥ 80 % of the ~28 gold views from `src/ingestion/scripts/migrations/20260422000000_gold-views.sql` have ≥ 1 E2E test by 2026-Q3 (Baseline: 0; Target: 80 %)
- 0 production regressions in covered metric definitions over rolling 90 days, once a view is covered (Baseline: not measured; Target: 0)
- First-time session start ≤ 60 s on a developer laptop with warm Docker cache (Baseline: N/A; Target: 60 s)
- Per-test wall-clock time after the session is warm: p50 ≤ 5 s, p95 ≤ 15 s (Baseline: N/A; Target: 5/15)

**Capabilities**:

- Author a test as `fixtures/<name>/bronze/*.csv` + `spec.yaml` + `expected/response.csv`
- Seed only the affected bronze tables from CSVs; never run Airbyte
- Run a dbt selector that builds exactly the silver/staging models needed by the test
- Query the analytics-api over HTTP loopback and compare the response items against an expected CSV with a cell-precise diff
- Regenerate expected CSVs from actual output via an explicit flag (golden update — gated behind a follow-on FEATURE)
- Parallelize via `pytest-xdist` with worker-scoped namespace suffixes

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Fixture | A folder under `src/ingestion/tests/e2e/fixtures/<test_name>/` containing `bronze/*.csv`, `spec.yaml`, and `expected/response.csv`. The unit a developer authors. |
| Bronze input CSV | A CSV file mapping 1-to-1 onto a bronze table (`bronze_<connector>.<entity>`). First row = column names; empty cell = SQL NULL. |
| Expected response CSV | A CSV file matching the JSON shape of `analytics-api` response `items[]` for the test's request, flattened to columns. |
| Spec YAML | Per-test config: which API endpoint, request body, dbt selector, key columns for sort/diff, float tolerance. |
| Gold view | A ClickHouse `VIEW` defined in `src/ingestion/scripts/migrations/*.sql` (e.g. `insight.people`). Read by analytics-api. |
| Test rig | The reusable runtime: docker compose, pytest fixtures, dbt runner, API client, CSV asserter. |
| Session fixture | A pytest fixture with `scope="session"` — runs once per `pytest` invocation across all tests. |
| Worker namespace | A suffix (e.g. `_w0`, `_w1`) added to schemas/tables when `pytest-xdist` runs tests in parallel. |
| Vertical slice | One end-to-end working test (`feature-csv-rig`). The MVP. |
| Golden update | Mode that rewrites every test's `expected/response.csv` from the actual API response. Gated behind an explicit flag. |

## 2. Actors

### 2.1 Human Actors

#### Data Engineer

**ID**: `cpt-bronze-to-api-e2e-actor-data-engineer`

**Role**: Authors and maintains dbt models, silver projections, and migration views. Adds tests when introducing new metrics or changing existing ones.
**Needs**: A declarative, folder-based way to express "given this bronze input, my gold view should produce this row". Wants to run a single test in under 10 s once the session is warm.

#### Backend Developer

**ID**: `cpt-bronze-to-api-e2e-actor-backend-developer`

**Role**: Changes analytics-api code (metric query builder, OData filter parser, ClickHouse SQL emission, MariaDB metric definitions). Adds regression tests on PRs.
**Needs**: A test rig that observes the HTTP response shape, not just the dbt or SQL output. Wants to run only the affected slice without rebuilding the whole pipeline.

#### Test Author (covering both roles above)

**ID**: `cpt-bronze-to-api-e2e-actor-test-author`

**Role**: Whoever is currently authoring a new fixture folder. May be a data engineer or backend developer.
**Needs**: A clear, single contract for what to put in `bronze/`, `spec.yaml`, and `expected/`. Cell-precise diff output when a test fails. A snapshot-update mode for the initial expected-CSV generation.

### 2.2 System Actors

#### CI Pipeline

**ID**: `cpt-bronze-to-api-e2e-actor-ci-pipeline`

**Role**: Runs the suite on every PR that touches `src/ingestion/`, `src/backend/services/analytics-api/`, or `src/ingestion/scripts/migrations/`. Reports pass/fail and, on failure, includes the cell-precise diff in the job output.

#### dbt CLI

**ID**: `cpt-bronze-to-api-e2e-actor-dbt-cli`

**Role**: Subprocess invoked by the test rig with selectors (e.g. `+silver_people+`). Owns DDL of staging/silver models.

#### analytics-api Binary

**ID**: `cpt-bronze-to-api-e2e-actor-analytics-api`

**Role**: Service under test. Spawned once per pytest session, bound to a loopback HTTP port. Reads from the test ClickHouse instance.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Requires Docker (Engine ≥ 24) for the compose stack (ClickHouse + MariaDB)
- Requires Python ≥ 3.12 (matches `dbt` runtime in the repo)
- Requires `cargo` toolchain to build the `analytics-api` binary once per session
- Requires ClickHouse and MariaDB versions pinned to production parity — the framework MUST NOT silently downgrade to an older container image
- Tests cannot run inside K8s — the framework is local-host only; for K8s integration use the local gitops deploy (`cd deploy/gitops && make deploy ENV=local`)

## 4. Scope

### 4.1 In Scope

- Bronze seeding via CSV → ClickHouse direct INSERT
- dbt staging and silver model execution with selectors
- ClickHouse migration view application (the ~28 gold views)
- analytics-api HTTP endpoint invocation: `POST /v1/metrics/{id}/query`, `GET /v1/columns`, `GET /v1/columns/{table}`, and any other endpoint declared in a test's `spec.yaml`
- CSV-based assertion of response payload with pandas-driven cell-precise diff
- Per-test isolation via TRUNCATE between tests; per-worker isolation via namespace suffix
- Parallel execution via `pytest-xdist`
- A snapshot-update mode for regenerating `expected/response.csv` (gated behind a follow-on FEATURE)
- CI integration as a GitHub Actions job (gated behind a follow-on FEATURE)

### 4.2 Out of Scope

- **Airbyte source connectors and sync correctness**: bronze is seeded by direct CSV INSERT, never through Airbyte. Airbyte connector bugs are out of this framework's coverage and belong to manual / staging tests.
- **Argo / Kestra workflow orchestration**: the framework calls dbt directly. Workflow definitions are tested separately.
- **Frontend rendering**: the framework asserts on the API JSON body. Frontend bullet rendering, threshold colors, etc. are out of scope and belong to frontend integration tests.
- **identity-service deep paths**: `GET /v1/persons/{email}` (deprecated) and `POST /v1/profiles` flows that depend on the identity service are out of v1 — when a test needs identity data it MUST inject pre-resolved data directly into bronze or MariaDB.
- **Multi-tenant fanout**: v1 runs single-tenant per test. Tenant-fanout scenarios (parallel tenants on the same gold view) belong to a follow-on FR after this framework stabilizes.
- **Performance / load testing**: this framework asserts correctness, not throughput. Load tests live elsewhere.
- **Connector descriptor validation**: validating that a connector YAML correctly declares its bronze schema is handled by the existing `source.sh validate` tool.
- **K8s deployment**: see §3.1.

## 5. Functional Requirements

> **Testing strategy**: This framework IS the testing surface. Its FRs are verified by meta-tests (smoke fixtures that exercise the framework itself) plus the test fixtures the framework runs.

### 5.1 Bronze Seeding

#### Seed bronze tables from per-test CSVs

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-fr-bronze-seed-from-csv`

The system **MUST** read each `fixtures/<test>/bronze/<schema>.<table>.csv` and INSERT its rows into the corresponding ClickHouse bronze table before dbt or any other downstream stage runs. First row of the CSV MUST be column names; empty cells MUST be inserted as SQL NULL.

**Rationale**: This is the entry point of the framework — the contract that lets a developer say "what the connector would have written" without running Airbyte.
**Actors**: `cpt-bronze-to-api-e2e-actor-test-author`, `cpt-bronze-to-api-e2e-actor-dbt-cli`

#### Truncate bronze tables between tests

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-fr-bronze-truncate`

Between tests the system **MUST** TRUNCATE the bronze tables that the current test touched (and only those). The system **MUST NOT** drop bronze tables (migrations create them once per session).

**Rationale**: Isolation without paying the cost of re-creating tables or running migrations.
**Actors**: `cpt-bronze-to-api-e2e-actor-ci-pipeline`

### 5.2 dbt Execution

#### Run a scoped dbt build per test

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-fr-dbt-run-scoped`

The system **MUST** invoke `dbt build` with a selector taken from `spec.yaml` (`dbt_selector`, e.g. `+silver_people+`) so only the staging/silver models required by the current test execute. The dbt manifest **MUST** be parsed once per session and reused via `--defer --state target/`.

**Rationale**: Running the full dbt graph for every test would push per-test latency well past the NFR.
**Actors**: `cpt-bronze-to-api-e2e-actor-dbt-cli`, `cpt-bronze-to-api-e2e-actor-data-engineer`

### 5.3 Migration Views

#### Apply ClickHouse migration views once per session

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-fr-gold-view-queried`

The system **MUST** apply all SQL files in `src/ingestion/scripts/migrations/*.sql` (in lexical order) against the test ClickHouse instance once at session start. Subsequent tests **MUST** read gold views via the analytics-api without re-applying migrations.

**Rationale**: Migrations are idempotent (`CREATE OR REPLACE VIEW`); they take a few seconds and should run once. Per-test re-application would violate the per-test latency NFR.
**Actors**: `cpt-bronze-to-api-e2e-actor-dbt-cli`

### 5.4 API Roundtrip

#### Invoke the analytics-api over HTTP loopback

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-fr-api-roundtrip`

The system **MUST** spawn the `analytics-api` binary once per pytest session, bind it to a random loopback port, and route each test's request — described by `spec.yaml` (`endpoint`, `method`, `request_body`) — to that port via HTTP. The system **MUST** disable auth in the spawned binary (test fixtures inject `SecurityContext` directly).

**Rationale**: HTTP-loopback round-trip is the contract the UI consumes. Skipping the HTTP layer (e.g. by linking against the axum router) would not catch regressions in router setup, serialization, or auth-middleware short-circuits.
**Actors**: `cpt-bronze-to-api-e2e-actor-analytics-api`, `cpt-bronze-to-api-e2e-actor-backend-developer`

### 5.5 Assertion

#### Diff API response items against the expected CSV

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-fr-csv-assert`

The system **MUST** compare the API response `items[]` against `fixtures/<test>/expected/response.csv` with the following semantics:
- The set of column names MUST match exactly (additive columns in either side fail the test).
- Rows are sorted by the `key_columns` declared in `spec.yaml` before comparison; row order in the JSON or CSV is otherwise ignored.
- Numeric columns are compared with absolute tolerance `float_tolerance` (default `1e-6`) from `spec.yaml`; non-numeric columns require exact match.
- On failure the system **MUST** render the first 20 mismatched cells with `(key, column, expected, actual)` in the pytest report.

**Rationale**: Per-test failure must point a developer at the specific cell that diverged. Diff output that says "responses differ" is not actionable.
**Actors**: `cpt-bronze-to-api-e2e-actor-test-author`, `cpt-bronze-to-api-e2e-actor-ci-pipeline`

### 5.6 Isolation

#### Isolate concurrent tests with per-worker namespaces

- [ ] `p2` - **ID**: `cpt-bronze-to-api-e2e-fr-test-isolation`

When tests run under `pytest-xdist`, the system **MUST** suffix every bronze schema, staging table, and silver table with the worker ID (`_w0`, `_w1`, …) so concurrent workers do not collide on shared rows.

**Rationale**: Without per-worker namespaces, parallel runs see each other's bronze data and tests flake.
**Actors**: `cpt-bronze-to-api-e2e-actor-ci-pipeline`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Cold session startup

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-nfr-cold-start`

The system **MUST** complete cold session startup — `docker compose up`, ClickHouse migrations applied, `analytics-api` binary built and spawned, dbt manifest parsed — within **60 s** on a developer laptop with a warm Docker image cache. With a cold Docker image cache the budget is **180 s**.

**Threshold**: 60 s warm cache; 180 s cold cache; measured wall-clock from `pytest` invocation to first test ready to run.
**Rationale**: A multi-minute startup makes one-test-at-a-time iteration impractical. 60 s is the threshold below which developers will choose to keep the session warm rather than redeploy.

#### Per-test latency

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-nfr-per-test-latency`

After session warm-up, per-test wall-clock time **MUST** be ≤ 5 s at p50 and ≤ 15 s at p95.

**Threshold**: p50 ≤ 5 s, p95 ≤ 15 s; measured per-test in CI on the standard runner.
**Rationale**: At p50 ≤ 5 s, a suite of 50 tests completes in well under five minutes — fast enough to run on every PR.

#### Parallel safety

- [ ] `p2` - **ID**: `cpt-bronze-to-api-e2e-nfr-parallel-safe`

The system **MUST** run safely under `pytest -n auto` (xdist). Two tests reading the same gold view in parallel workers **MUST NOT** see each other's bronze data.

**Threshold**: zero cross-worker contamination across 100 randomized parallel runs of two reference tests.
**Rationale**: CI runners have multiple cores; serial execution wastes capacity.

#### Diff readability

- [ ] `p2` - **ID**: `cpt-bronze-to-api-e2e-nfr-diff-readability`

On failure, the system **MUST** render the cell-precise diff in the pytest captured output (not only in a log file). Each entry **MUST** include the row key, column name, expected value, and actual value.

**Threshold**: at least the first 20 mismatched cells visible in the pytest captured stdout; full diff available on disk.
**Rationale**: A developer reading the CI log should not have to download an artifact to see why the test failed.

### 6.2 NFR Exclusions

- **Project-default availability NFR**: Not applicable — the framework is dev-time only and does not need a production SLA.
- **Project-default security NFR (auth)**: Not applicable — the spawned `analytics-api` runs with auth disabled; the framework MUST NOT expose its loopback port outside `127.0.0.1`.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Pytest entry point

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-interface-pytest-entry`

**Type**: pytest plugin (auto-loaded via `conftest.py` under `src/ingestion/tests/e2e/`)
**Stability**: unstable (v1)
**Description**: The default `pytest src/ingestion/tests/e2e/` invocation discovers every fixture folder and emits one test per fixture. Standard pytest markers / `-k` / `-n` work.
**Breaking Change Policy**: Major version bump required to change fixture-folder layout or `spec.yaml` schema.

#### Fixture folder layout

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-interface-fixture-layout`

**Type**: Filesystem convention
**Stability**: unstable (v1)
**Description**: Each fixture is a folder containing `bronze/<schema>.<table>.csv` files, a `spec.yaml`, and `expected/response.csv`. `spec.yaml` keys: `endpoint`, `method` (default POST), `metric_id` (UUID, when applicable), `request_body`, `dbt_selector`, `key_columns`, `float_tolerance` (default `1e-6`).

### 7.2 External Integration Contracts

#### ClickHouse bronze schema contract

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-contract-bronze-schema`

**Direction**: required from the framework consumer (test author)
**Protocol/Format**: ClickHouse table schema as defined by the existing connector descriptors and migrations
**Compatibility**: forward-compatible additive columns OK; the CSV asserter MUST not require a CSV to list every column — missing columns inserted as NULL.

#### analytics-api response shape contract

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-contract-api-response`

**Direction**: provided by the `analytics-api` service; consumed by the asserter
**Protocol/Format**: `application/json`, `{"items": [...], "page_info": {...}}` per `POST /v1/metrics/{id}/query`
**Compatibility**: any change to the response shape is a versioned breaking change to the framework.

## 8. Use Cases

#### UC-001 Author a new test against a gold view

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-usecase-author-test`

**Actor**: `cpt-bronze-to-api-e2e-actor-test-author`

**Preconditions**:
- The gold view exists in `src/ingestion/scripts/migrations/*.sql`
- The metric definition exists in MariaDB seed (or a `metric_id` fixture row is included)

**Main Flow**:
1. Author creates `fixtures/<name>/` folder
2. Author writes `bronze/<schema>.<table>.csv` for every bronze table the dbt selector will read
3. Author writes `spec.yaml` with endpoint, request body, dbt selector, key columns
4. Author runs `pytest --update-snapshots src/ingestion/tests/e2e/fixtures/<name>` to generate `expected/response.csv` from the live response
5. Author inspects `expected/response.csv` and adjusts the bronze inputs until it reflects the intended scenario
6. Author commits the folder

**Postconditions**:
- A new test runs on every CI invocation
- The test fails if any of the four authoring surfaces drifts from the expected payload

**Alternative Flows**:
- **No metric_id in MariaDB**: spec.yaml MUST include an inline metric definition that the framework inserts into MariaDB before the request

#### UC-002 Diagnose a failing test

- [ ] `p2` - **ID**: `cpt-bronze-to-api-e2e-usecase-diagnose-failure`

**Actor**: `cpt-bronze-to-api-e2e-actor-data-engineer`

**Preconditions**: CI has reported a failing test

**Main Flow**:
1. Developer reads the cell-precise diff in the CI log: rows of `(key, column, expected, actual)`
2. Developer runs the same test locally with `pytest -k <name>`
3. Developer inspects intermediate state: silver tables, gold view output, raw API response
4. Developer either fixes the upstream code, or — if the expected payload was wrong — regenerates the snapshot

**Postconditions**: regression understood, fix or snapshot update committed

**Alternative Flows**:
- **Flake suspected**: developer reruns the test 10x; if it fails ≥ 1 time the isolation NFR is violated and a fix is opened against the framework, not the test.

## 9. Acceptance Criteria

- [ ] A developer can author a passing test as a single folder under `src/ingestion/tests/e2e/fixtures/` without touching framework code
- [ ] Cold session startup completes within 60 s on warm cache, 180 s on cold cache
- [ ] At least one full fixture exists, exercising `insight.people` end-to-end, and passes
- [ ] `pytest -n auto` runs the smoke suite without cross-worker contamination over 100 randomized runs
- [ ] On a forced failure (e.g., breaking a silver model), the cell-precise diff appears in the pytest captured stdout
- [ ] CI integration job runs the suite on every PR that touches `src/ingestion/` or `src/backend/services/analytics-api/`

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Docker (Engine ≥ 24) | Hosts ClickHouse and MariaDB containers | p1 |
| ClickHouse 24.x image | Same major version as production | p1 |
| MariaDB 11.x image | Same major version as production | p1 |
| Python ≥ 3.12 | Runs pytest, dbt subprocess, pandas asserter | p1 |
| `cargo` toolchain | Builds `analytics-api` binary once per session | p1 |
| dbt project at `src/ingestion/dbt/` | The unit being exercised | p1 |
| ClickHouse migrations at `src/ingestion/scripts/migrations/` | Define the gold views | p1 |
| `analytics-api` Cargo workspace member | Service under test | p1 |
| Metric Catalog (MariaDB `analytics.metrics`) | Defines the queries the API executes | p1 |
| pandas ≥ 2.2 | Cell-precise CSV diff | p1 |
| `pytest-xdist` | Parallel execution | p2 |

## 11. Assumptions

- ClickHouse `CREATE OR REPLACE VIEW` semantics in the migration set are idempotent and safe to run against a fresh schema with bronze tables already created.
- The `analytics-api` binary supports an auth-disabled mode suitable for local/test use.
- Per-tenant scoping in production is enforced by `SecurityContext` injection; the test framework either bypasses tenancy (single-tenant tests) or seeds it into the spawned process directly.
- dbt's `--defer --state target/` works when the test-side manifest is built once per session and pointed back to the prior-state directory.
- `pytest-xdist` worker IDs are stable within a single test session and can be safely embedded into ClickHouse schema names.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| dbt selector behavior changes across dbt versions and breaks `--defer --state` reuse | Per-test latency NFR violated; suite slows to minutes | Pin dbt version in `pyproject.toml` / `requirements.txt`; rerun NFR check on dbt upgrade |
| analytics-api binary spawn becomes slow as it accumulates dependencies | Cold session start NFR violated | Build with `--release`, cache target dir between CI runs; add explicit cargo-cache step in CI |
| ClickHouse `TRUNCATE` between tests does not actually release storage and disk fills | Long suites OOM the disk on CI runners | Add a session-end `OPTIMIZE TABLE ... FINAL` + observe disk usage; document a periodic full-volume wipe |
| Float comparison tolerance hides real regressions in aggregate metrics | False negatives — test passes when number is subtly wrong | Default tolerance `1e-6`; require per-test override only with a justification comment |
| Fixture folder format drifts as the framework evolves; old fixtures stop running | Maintenance debt — every change forces a sweep of all fixtures | Version `spec.yaml` (`spec_version: 1`); the runner refuses unknown major versions; minor changes are additive |
| Migrations require live MariaDB writes (e.g. seed inserts) that conflict with parallel workers | xdist flakes | All MariaDB writes go through a per-worker schema or per-row tenant prefix |
| Identity-service dependency creeps in via gold views that join MariaDB | Out-of-scope coupling | Document in §4.2; framework MUST refuse to start if a fixture references a path that requires identity-service |
