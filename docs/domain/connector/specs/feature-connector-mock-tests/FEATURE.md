---
status: proposed
date: 2026-07-09
---

# Feature: Connector Mock-Server Tests

- [ ] `p1` - **ID**: `cpt-insightspec-featstatus-connector-mock-tests`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Author and Run Mock Tests for a Connector](#author-and-run-mock-tests-for-a-connector)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Build In-Process Source from a Connector Package](#build-in-process-source-from-a-connector-package)
  - [Execute a Mock Read](#execute-a-mock-read)
- [4. States (CDSL)](#4-states-cdsl)
  - [Mock Test Lifecycle](#mock-test-lifecycle)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Test Level Ladder](#test-level-ladder)
  - [Shared Test Harness](#shared-test-harness)
  - [Per-Connector Test Layout](#per-connector-test-layout)
  - [Stream Coverage Matrix](#stream-coverage-matrix)
  - [Schema Conformance Assertions](#schema-conformance-assertions)
  - [Determinism and Isolation](#determinism-and-isolation)
  - [Runner and CI Integration](#runner-and-ci-integration)
  - [Reference Implementation (task-tracking/jira)](#reference-implementation-task-trackingjira)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

Introduce credential-free, CI-runnable **mock-server tests** for Insight Connectors, closing the gap between static manifest validation (`source.sh validate-strict` / `validate`) and live smoke tests (`check` / `discover` / `read` with tenant credentials). A mock test loads the connector's `connector.yaml` **in-process** through the pinned `airbyte-cdk`, intercepts HTTP at the transport layer (`airbyte_cdk.test.mock_http.HttpMocker`), runs a full protocol `read` as a black box (`airbyte_cdk.test.entrypoint_wrapper.read`), and asserts on the emitted records, state, and logs.

This is the pattern used by certified upstream Airbyte connectors (e.g. `source-jira`'s `unit_tests/mock_server/`, one test module per stream), adapted to our repo layout: tests live inside the connector package (`src/ingestion/connectors/{category}/{name}/tests/`), and shared rig code lives in one central harness package.

The tests verify the behavior that static validation cannot see and live smoke tests cannot verify deterministically:

- pagination stop conditions across multiple pages;
- incremental cursor: state emission and request filtering on resume;
- error-handler policy (retry on 429/5xx, ignored status codes produce no `ERROR` log);
- `record_filter` and `AddFields` transformations — in particular the mandatory `tenant_id` / `source_id` / `unique_key` stamping from `insight_tenant_id` / `insight_source_id` config;
- substream partitioning (one child request per parent partition);
- record shape against the stream schema (`schemas/<stream>.json` when generated, else the manifest inline schema).

Airbyte's Connector Acceptance Tests (CAT) are deliberately **not** adopted: CAT requires a Docker image per connector and live sandbox credentials, which our Level 2 smoke ladder already covers with `source.sh`. Mock tests take CAT's role of protocol-behavior verification without the credential dependency.

### 1.2 Purpose

Today a manifest change (pagination expression, cursor format, error handler, transformation) is verified only by running against a live source API — slow, credential-gated, non-deterministic, and unavailable in CI. Regressions such as a broken stop condition or a dropped `AddFields` block reach dev unnoticed. Mock tests make connector behavior a mandatory, deterministic CI gate and give connector authors a fast local loop (`pytest`, seconds, no Docker, no credentials).

**Requirements**:

- `cpt-insightspec-fr-cn-connector-sdk` *(the "local testing capabilities" of the SDK)*
- `cpt-insightspec-fr-cn-connector-spec`
- `cpt-insightspec-fr-cn-incremental-sync`
- `cpt-insightspec-fr-cn-idempotent-extraction`
- `cpt-insightspec-nfr-cn-rate-limit-compliance` *(retry/backoff behavior verified against a mock returning 429)*

**Principles**:

- `cpt-insightspec-principle-cn-declarative-first` *(the manifest is the unit under test; no test doubles for CDK internals)*
- `cpt-insightspec-principle-cn-self-contained-package` *(tests ship inside the connector package)*
- `cpt-insightspec-principle-cn-mandatory-tenant-source`
- `cpt-insightspec-principle-cn-schema-from-real-data` *(fixtures are real API response shapes; records must conform to `schemas/`)*

### 1.3 Actors

| Actor | Role in Feature |
|-------|-----------------|
| `cpt-insightspec-actor-cn-connector-author` | Authors `tests/` in the connector package, runs `pytest` locally |
| `cpt-insightspec-actor-cn-platform-engineer` | Maintains the shared harness and the CI gate |
| `cpt-insightspec-actor-cn-source-api` | Impersonated by `HttpMocker` fixtures; never contacted |

### 1.4 References

- **PRD**: [../PRD.md](../PRD.md)
- **DESIGN**: [../DESIGN.md](../DESIGN.md) — §3.6 "Local Debugging with source.sh" (Levels 0 and 2 of the ladder)
- **Prior art**: Airbyte `source-jira/unit_tests/mock_server/` (builder pattern, per-stream modules), `airbyte_cdk.test` public API
- **Related**: [Declarative YAML Test Rig](../../../bronze-to-api-e2e/specs/feature-yaml-rig/FEATURE.md) (Level 3 — pipeline e2e; out of scope here)
- **Use case**: `cpt-insightspec-usecase-cn-develop-connector`

## 2. Actor Flows (CDSL)

### Author and Run Mock Tests for a Connector

- [ ] `p1` - **ID**: `cpt-insightspec-flow-cn-mock-test-author-and-run`

**Actor**: `cpt-insightspec-actor-cn-connector-author`

**Success Scenarios**:

- Author adds a stream to `connector.yaml`, writes `tests/test_<stream>.py` covering the matrix rows that apply, and the suite passes without credentials
- Author breaks a pagination stop condition; the multi-page test fails with expected-vs-actual record counts
- Author edits `AddFields`; the tenant-stamping test fails naming the missing field

**Error Scenarios**:

- A test config omits a field required by the manifest `spec` → source construction fails at test setup naming the field
- The connector makes an HTTP request no fixture matches → `HttpMocker` fails the test with the unmatched request (no silent network fallthrough)
- An emitted record violates the stream schema → schema-conformance assertion fails naming the stream, field, and violation

**Steps**:

1. [ ] - `p1` - Author creates `src/ingestion/connectors/{category}/{name}/tests/` with `config.py` (ConfigBuilder), optional `request_builder.py` / `response_builder.py`, and `fixtures/*.json` - `inst-mock-author-layout`
2. [ ] - `p1` - Author writes one `test_<stream>.py` per stream, selecting cases from `cpt-insightspec-dod-cn-mock-coverage-matrix` - `inst-mock-author-cases`
3. [ ] - `p1` - Author runs `pytest src/ingestion/connectors/{category}/{name}/tests/` - `inst-mock-author-run`
4. [ ] - `p1` - Algorithm: harness runs `cpt-insightspec-algo-cn-mock-source-build`, then per test `cpt-insightspec-algo-cn-mock-read` - `inst-mock-author-invoke`
5. [ ] - `p1` - **IF** all assertions hold **RETURN** "ok" - `inst-mock-author-pass`
6. [ ] - `p1` - **ELSE** pytest reports the failing stream, case, and expected-vs-actual - `inst-mock-author-fail`

## 3. Processes / Business Logic (CDSL)

### Build In-Process Source from a Connector Package

- [ ] `p1` - **ID**: `cpt-insightspec-algo-cn-mock-source-build`

**Input**: connector package path `pkg` (contains `connector.yaml`), synthetic `config` dict

**Output**: a runnable CDK source object | error

**Steps**:

1. [ ] - `p1` - Load `pkg/connector.yaml` as the declarative manifest (no `$ref` preprocessing — same bytes Airbyte receives) - `inst-msrc-load`
2. [ ] - `p1` - Validate `config` against the manifest `spec`; **IF** a required field is missing → error naming the field - `inst-msrc-spec-check`
3. [ ] - `p1` - Instantiate the source via the CDK's declarative-manifest entry point (`ConcurrentDeclarativeSource`), the same code path as the `source-declarative-manifest` image - `inst-msrc-instantiate`
4. [ ] - `p1` - **RETURN** the source; construction failure surfaces the CDK error verbatim - `inst-msrc-return`

### Execute a Mock Read

- [ ] `p1` - **ID**: `cpt-insightspec-algo-cn-mock-read`

**Input**: source, `config`, stream name, sync mode, optional input `state`, list of `(request_matcher, response | [responses])` fixtures

**Output**: `EntrypointOutput` (records, state messages, logs) | test failure

**Steps**:

1. [ ] - `p1` - Freeze wall clock (`freezegun`) so cursor arithmetic and datetime-templated request params are deterministic - `inst-mread-freeze`
2. [ ] - `p1` - Register every fixture pair on `HttpMocker`; a list of responses serves consecutive calls (pagination) - `inst-mread-register`
3. [ ] - `p1` - Build a configured catalog for the stream and sync mode (`CatalogBuilder`) - `inst-mread-catalog`
4. [ ] - `p1` - Run `entrypoint_wrapper.read(source, config, catalog, state)` — full protocol read, not a unit call into stream classes - `inst-mread-read`
5. [ ] - `p1` - **IF** the connector issued a request matched by no fixture → fail with the unmatched request - `inst-mread-unmatched`
6. [ ] - `p1` - **RETURN** typed output: `output.records`, `output.state_messages`, `output.logs`, `output.errors` - `inst-mread-return`

## 4. States (CDSL)

### Mock Test Lifecycle

- [ ] `p1` - **ID**: `cpt-insightspec-state-cn-mock-test-lifecycle`

**States**: `PENDING`, `BUILDING_SOURCE`, `MOCKING`, `READING`, `ASSERTING`, `PASSED`, `FAILED`

**Initial State**: `PENDING`

**Transitions**:

1. [ ] - `p1` - **FROM** `PENDING` **TO** `BUILDING_SOURCE` **WHEN** the test starts - `inst-mstate-pending-building`
2. [ ] - `p1` - **FROM** `BUILDING_SOURCE` **TO** `MOCKING` **WHEN** the manifest loads and config validates - `inst-mstate-building-mocking`
3. [ ] - `p1` - **FROM** `MOCKING` **TO** `READING` **WHEN** all fixtures are registered - `inst-mstate-mocking-reading`
4. [ ] - `p1` - **FROM** `READING` **TO** `ASSERTING` **WHEN** the protocol read completes - `inst-mstate-reading-asserting`
5. [ ] - `p1` - **FROM** `ASSERTING` **TO** `PASSED` **WHEN** every assertion holds - `inst-mstate-asserting-passed`
6. [ ] - `p1` - **FROM** any state **TO** `FAILED` **WHEN** its guard fails (unmatched request, CDK error, assertion) - `inst-mstate-any-failed`

## 5. Definitions of Done

### Test Level Ladder

- [ ] `p1` - **ID**: `cpt-insightspec-dod-cn-test-ladder`

The connector test ladder **MUST** be documented and ordered as follows; each level is a precondition for the next:

| Level | What | Tooling | Credentials | Where it runs |
|---|---|---|---|---|
| L0 | Static manifest validation | `source.sh validate-strict` → `validate` | no | local + CI |
| L1 | **Mock-server tests (this feature)** | `pytest` + `airbyte_cdk.test` | no | local + CI (mandatory gate) |
| L2 | Live smoke | `source.sh check` / `discover` / `read <tenant>` | yes | local, pre-deploy |
| L3 | Pipeline e2e (bronze → API) | metrics YAML rig | no (seeded) | local + CI (separate suite) |

**Implements**: `cpt-insightspec-flow-cn-mock-test-author-and-run`

**Touches**: `src/ingestion/tools/declarative-connector/README.md` (ladder update), `docs/domain/connector/README.md`

### Shared Test Harness

- [ ] `p1` - **ID**: `cpt-insightspec-dod-cn-mock-harness`

A single installable package `src/ingestion/tests/connectors/` **MUST** provide: `get_source(pkg_path, config)` implementing `cpt-insightspec-algo-cn-mock-source-build`; re-exports of `HttpMocker`, `HttpRequest`, `HttpResponse`, `CatalogBuilder`, `entrypoint_wrapper.read`; a base `ConfigBuilder` that always carries `insight_tenant_id` / `insight_source_id`; a `read_stream(...)` convenience implementing `cpt-insightspec-algo-cn-mock-read`; and a schema-conformance asserter (see `cpt-insightspec-dod-cn-mock-schema-conformance`). Its `pyproject.toml` **MUST** pin `airbyte-cdk` to the line matching the `version:` header of the connector manifests (currently `6.60.x`); the pin **MUST** be updated in lockstep with manifest `version:` bumps.

**Implements**: `cpt-insightspec-algo-cn-mock-source-build`, `cpt-insightspec-algo-cn-mock-read`

**Touches**: `src/ingestion/tests/connectors/pyproject.toml`, `src/ingestion/tests/connectors/connector_tests/*.py`

### Per-Connector Test Layout

- [ ] `p1` - **ID**: `cpt-insightspec-dod-cn-mock-layout`

Tests **MUST** live inside the connector package as `src/ingestion/connectors/{category}/{name}/tests/` containing: `test_<stream>.py` (one module per stream under test), `config.py` (connector-specific ConfigBuilder extending the shared base), optional `request_builder.py` / `response_builder.py` for connectors with many streams, and `fixtures/*.json` holding realistic response bodies (field names and shapes taken from real API responses, values synthetic — no real customer data, tokens, or hostnames). The package **MUST** remain self-contained: no imports from other connector packages.

**Implements**: `cpt-insightspec-flow-cn-mock-test-author-and-run`

**Principles**: `cpt-insightspec-principle-cn-self-contained-package`

**Touches**: `src/ingestion/connectors/{category}/{name}/tests/`

### Stream Coverage Matrix

- [ ] `p1` - **ID**: `cpt-insightspec-dod-cn-mock-coverage-matrix`

For every stream listed in `descriptor.yaml`, the suite **MUST** cover each matrix row whose precondition the manifest satisfies:

| Case | Required when | Asserts |
|---|---|---|
| `full_refresh_single_page` | always | record count and key fields from one fixture page |
| `schema_conformance` | always | every emitted record validates against the stream schema |
| `tenant_source_stamping` | always | `tenant_id`, `source_id`, `unique_key` equal the `insight_*` config-derived values |
| `empty_page` | always | 0 records, no `ERROR` log |
| `pagination_multi_page` | stream declares a paginator | all pages read; stop condition halts exactly at the last page |
| `incremental_state` | stream declares `incremental_sync` | a state message is emitted with the expected cursor; a second read given that state issues a request filtered from the cursor |
| `substream_partition` | stream has a parent stream | one child request per parent partition; child records carry partition context |
| `record_filter` / `transformations` | manifest declares them | filtered records absent; added/renamed fields present |
| `error_retry` | always | a 429 (or retryable 5xx) response is retried per the error handler and the read succeeds without record loss |
| `error_ignore` | manifest ignores status codes | the ignored status yields 0 records and no `ERROR` log |

A row skipped despite a satisfied precondition **MUST** carry an explicit skip reason in the test module (mirror of CAT's `bypass_reason`).

**Implements**: `cpt-insightspec-algo-cn-mock-read`

**Requirements**: `cpt-insightspec-fr-cn-incremental-sync`, `cpt-insightspec-nfr-cn-rate-limit-compliance`

### Schema Conformance Assertions

- [ ] `p1` - **ID**: `cpt-insightspec-dod-cn-mock-schema-conformance`

The harness **MUST** provide `assert_records_conform(records, pkg_path, stream)` validating every emitted record against the stream schema, resolved as `schemas/<stream>.json` when generated, else the stream's InlineSchemaLoader schema from connector.yaml. A type mismatch **MUST** fail the test naming the stream, field, and offending value; in the default strict mode a record field absent from the schema fails too (manifest↔schema drift), with `strict=False` as the documented escape for streams that intentionally pass through undeclared source fields. This keeps schemas (produced from real data) authoritative and catches drift before dbt sees it.

**Implements**: `cpt-insightspec-algo-cn-mock-read`

**Principles**: `cpt-insightspec-principle-cn-schema-from-real-data`

**Touches**: `src/ingestion/tests/connectors/connector_tests/schema_assert.py`

### Determinism and Isolation

- [ ] `p1` - **ID**: `cpt-insightspec-dod-cn-mock-determinism`

Mock tests **MUST** be deterministic and offline: wall clock frozen via `freezegun` in every test touching cursors or datetime-templated params; all HTTP intercepted by `HttpMocker` — an unmatched request fails the test rather than reaching the network; no credentials, secrets, or `connections/` tenant configs read; no Docker required. Test configs are synthetic and satisfy the manifest `spec` only.

**Implements**: `cpt-insightspec-algo-cn-mock-read`

### Runner and CI Integration

- [ ] `p1` - **ID**: `cpt-insightspec-dod-cn-mock-ci`

`pytest src/ingestion/connectors/{category}/{name}/tests/` **MUST** run a single connector's suite; a bare `pytest` in the harness root **MUST** run the harness's own tests plus every nocode connector's suite (CDK connectors are excluded — they carry their own pyproject, CDK pin, and coverage component). The `/connector test <name>` skill command **MUST** run L0 (`validate-strict`, `validate`) followed by L1 (mock tests) and report per-level results.

CI integration goes through the shared coverage gate (`scripts/ci/components.py` + `scripts/ci/coverage.py`): the harness is registered as a `lang: python` component (`cov_package: connector_tests`) whose `paths` include both the harness directory and each covered nocode connector package, so a manifest or suite change re-runs the component (longest-prefix match keeps nested components like `jira-enrich` with their own jobs). Line coverage measures the harness package — declarative manifests have no first-party lines; a connector's **behavioral** coverage is the stream coverage matrix (`cpt-insightspec-dod-cn-mock-coverage-matrix`). The component is subject to the standard gates (≥ 80% overall, ≥ 80% new-code) and **MUST** block merge on failure.

**Implements**: `cpt-insightspec-dod-cn-test-ladder`

**Touches**: `.claude/skills/connector/SKILL.md`, `scripts/ci/components.py`

### Reference Implementation (task-tracking/jira)

- [ ] `p1` - **ID**: `cpt-insightspec-dod-cn-mock-reference`

The feature **MUST** ship one complete reference suite for `task-tracking/jira` covering every applicable matrix row for at least two streams — one plain paginated stream (`jira_projects`) and one incremental substream (`jira_issue_keys`) — including the `tenant_source_stamping` case asserting `unique_key = "{tenant}-{source}-{id}"`. The reference suite is the copy-from template for all subsequent connectors and is linked from the connector authoring guide.

**Implements**: `cpt-insightspec-flow-cn-mock-test-author-and-run`

**Touches**: `src/ingestion/connectors/task-tracking/jira/tests/`

## 6. Acceptance Criteria

- [ ] **Given** a fresh checkout with no credentials and no Docker, **When** a developer runs `pytest src/ingestion/connectors/task-tracking/jira/tests/`, **Then** the reference suite passes in under a minute
- [ ] **Given** the reference suite passes, **When** the paginator stop condition in `connector.yaml` is broken, **Then** `pagination_multi_page` fails with expected-vs-actual record counts
- [ ] **Given** the reference suite passes, **When** the `AddFields` block stamping `tenant_id` is removed, **Then** `tenant_source_stamping` and `schema_conformance` fail naming the missing field
- [ ] **Given** a test whose fixtures omit one endpoint, **When** the connector requests it, **Then** the test fails printing the unmatched request instead of touching the network
- [ ] **Given** an incremental stream test, **When** the read is re-run with the emitted state, **Then** the mocked request is asserted to carry the cursor-derived filter parameter
- [ ] **Given** a PR that only touches `src/ingestion/connectors/wiki/outline/`, **When** CI runs, **Then** only the outline mock suite (plus L0 validation) gates the merge
