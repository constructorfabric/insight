# Decomposition: Bronze-to-API E2E Test Framework

<!-- toc -->

- [1. Overview](#1-overview)
- [2. Entries](#2-entries)
  - [2.1 Test Rig Scaffolding ⏳ HIGH](#21-test-rig-scaffolding--high)
  - [2.2 Fixture Loader ⏳ HIGH](#22-fixture-loader--high)
  - [2.3 CSV Rig (vertical slice MVP) ⏳ HIGH](#23-csv-rig-vertical-slice-mvp--high)
  - [2.4 Dbt Runner ⏳ HIGH](#24-dbt-runner--high)
  - [2.5 API Spawner ⏳ HIGH](#25-api-spawner--high)
  - [2.6 ClickHouse Seeder & CSV Asserter ⏳ HIGH](#26-clickhouse-seeder--csv-asserter--high)
  - [2.7 Snapshot Update ⏳ MEDIUM](#27-snapshot-update--medium)
  - [2.8 CI Integration ⏳ MEDIUM](#28-ci-integration--medium)
- [3. Feature Dependencies](#3-feature-dependencies)
- [4. Coverage Matrix](#4-coverage-matrix)
- [5. Execution Order](#5-execution-order)

<!-- /toc -->

---

## 1. Overview

The DESIGN is decomposed into 8 features. The order is deliberate: a foundation feature (`scaffolding`) sets up docker compose, pytest layout, and session lifecycle; a vertical-slice feature (`csv-rig`) cuts through every layer end-to-end so the rest of the work has a working harness to land into; the remaining six features add depth to each component plus the polish needed for daily developer use and CI.

**Decomposition Strategy**:

- **Foundation + vertical slice first**: `scaffolding` and `csv-rig` ship together as the MVP. After they land, a developer can author a passing test against `insight.people` and the rest of the framework can be expanded incrementally without ever leaving the system in a broken state.
- **One feature per component, with the runtime cut out**: each non-foundation, non-MVP feature corresponds 1-to-1 to a DESIGN component (`fixture-loader`, `dbt-runner`, `api-spawner`, `csv-asserter`). Exception: `migration-applier` is small enough that it ships inside `scaffolding`.
- **Polish behind separate features**: `snapshot-update` (the `--update-snapshots` flag) and `ci-integration` (GitHub Actions) are explicitly separate from the MVP because both can land later without blocking developer use.
- **No circular dependencies**: dependency graph is a strict DAG with `scaffolding` at the root.
- **100% coverage**: every PRD FR and every DESIGN component appears in at least one feature's "Requirements Covered" / "Design Components" — verified by the matrix in §4.

**Late-Phase Items (deferred follow-on)**:

- Multi-tenant fanout fixtures (PRD §4.2)
- Performance / load testing — out of scope per PRD §4.2
- Identity-service end-to-end coverage — currently a hand-rolled MariaDB seed, will get its own FEATURE when the v1.x stabilizes

---

## 2. Entries

**Overall implementation status:**

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-status-overall`

### 2.1 [Test Rig Scaffolding](feature-test-rig-scaffolding/) ⏳ HIGH

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-feature-test-rig-scaffolding`

- **Purpose**: Establish the framework skeleton — repository layout under `src/ingestion/tests/e2e/`, the docker compose stack (ClickHouse + MariaDB), pytest configuration, session lifecycle (compose up/down, migration apply, MariaDB seed). After this lands, every other feature has a stable place to land.

- **Depends On**: None

- **Scope**:
  - Directory layout: `src/ingestion/tests/e2e/{compose,fixtures,lib,meta}`
  - `compose/docker-compose.yml` with ClickHouse 24.x + MariaDB 11.x pinned to production parity, ports on `127.0.0.1` only, randomized credentials per run
  - `conftest.py` with `pytest_sessionstart` hook orchestrating compose-up + healthcheck wait
  - `migration-applier` logic (small enough to live inline, not as a separate feature): `clickhouse-client --multiquery` over `src/ingestion/scripts/migrations/*.sql` in lexical order
  - MariaDB catalog seed: insert metric definitions read from a `tests/e2e/seed/metrics.yaml` (declarative)
  - Per-worker bronze-schema bootstrap (`bronze_<connector>_w{N}`)
  - Session teardown order

- **Out of scope**:
  - Reading or interpreting individual fixture folders (that's `fixture-loader`)
  - dbt manifest parsing (that's `dbt-runner`)
  - analytics-api binary build/spawn (that's `api-spawner`)
  - CSV diff (that's `csv-asserter`)
  - `--update-snapshots` CLI flag (that's `snapshot-update`)
  - GitHub Actions workflow file (that's `ci-integration`)

- **Requirements Covered**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-fr-gold-view-queried` (migration apply is part of scaffolding)
  - [ ] `p1` - `cpt-bronze-to-api-e2e-nfr-cold-start`
  - [ ] `p2` - `cpt-bronze-to-api-e2e-nfr-parallel-safe` (per-worker namespace setup)

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-principle-shared-session`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-principle-no-airbyte`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-constraint-version-parity`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-constraint-no-ddl-mutation`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-constraint-loopback-only`

- **Domain Model Entities**: `WorkerContext`

- **Design Components**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-component-migration-applier`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-component-session-rig`

- **API**: (internal tooling; no external API surface)

- **Sequences**:

  - `cpt-bronze-to-api-e2e-seq-session-startup`

- **Data**: (consumes existing bronze/silver/gold schemas)

### 2.2 [Fixture Loader](feature-fixture-loader/) ⏳ HIGH

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-feature-fixture-loader`

- **Purpose**: Read a `fixtures/<name>/` folder into a typed `Fixture` value: parse `spec.yaml` against a JSON Schema, enumerate `bronze/*.csv`, load `expected/response.csv` into a pandas DataFrame, validate `key_columns` exist in expected.

- **Depends On**: `cpt-bronze-to-api-e2e-feature-test-rig-scaffolding`

- **Scope**:
  - `spec.yaml` JSON Schema (`spec_version: 1`)
  - Folder discovery (one fixture per subfolder of `fixtures/`)
  - `expected/response.csv` load into DataFrame
  - Pydantic / dataclass `Fixture` value type
  - Validation errors raised at session-collect time (fail fast — not at test-run time)

- **Out of scope**:
  - Typed CSV-cell coercion (lives in `ch-seeder` because it needs `system.columns`)
  - Anything that writes to ClickHouse
  - Snapshot-update path

- **Requirements Covered**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-fr-bronze-seed-from-csv` (this feature parses the inputs; the seed feature inserts them)

- **Design Principles Covered**:

  - [ ] `p2` - `cpt-bronze-to-api-e2e-principle-fixtures-are-truth`

- **Design Components**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-component-fixture-loader`

- **Domain Model Entities**: `Fixture`, `SpecYaml`

- **API**: (internal Python module — `lib.fixture_loader.load(path) -> Fixture`)

- **Data**: (read-only filesystem)

### 2.3 [CSV Rig (vertical slice MVP)](feature-csv-rig/) ⏳ HIGH

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-feature-csv-rig`

- **Purpose**: One full end-to-end test path: load fixture → typed bronze INSERT → scoped `dbt build` → API call → cell-precise CSV diff. Ships with exactly one passing fixture (against `insight.people`). After this feature lands, the framework is usable; the remaining features are scaling and polish.

- **Depends On**:

  - `cpt-bronze-to-api-e2e-feature-test-rig-scaffolding`
  - `cpt-bronze-to-api-e2e-feature-fixture-loader`
  - `cpt-bronze-to-api-e2e-feature-dbt-runner`
  - `cpt-bronze-to-api-e2e-feature-api-spawner`
  - `cpt-bronze-to-api-e2e-feature-csv-asserter`

- **Scope**:
  - The integration glue (pytest fixtures with `scope="function"`) that wires `fixture-loader → ch-seeder → dbt-runner → api-client → csv-asserter` for one test
  - One reference fixture at `src/ingestion/tests/e2e/fixtures/people_smoke/`
  - The detailed CDSL flows / processes / state machine for the execution loop (in the FEATURE doc itself)
  - Per-test teardown using the `ch-seeder.truncate_touched()` ledger

- **Out of scope**:
  - `--update-snapshots` flag (`snapshot-update`)
  - CI workflow YAML (`ci-integration`)
  - More than one reference fixture (those land per-PR as developers cover their views)

- **Requirements Covered**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-fr-bronze-seed-from-csv`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-fr-bronze-truncate`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-fr-dbt-run-scoped`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-fr-gold-view-queried`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-fr-api-roundtrip`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-fr-csv-assert`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-nfr-per-test-latency`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-principle-shared-session`
  - [ ] `p2` - `cpt-bronze-to-api-e2e-principle-fixtures-are-truth`

- **Design Components**: (integrates all components below; primary owner is `session-rig`'s per-test fixture)

  - [ ] `p1` - `cpt-bronze-to-api-e2e-component-session-rig`

- **Domain Model Entities**: `Fixture`, `ApiResponse`, `AssertionResult`

- **Sequences**:

  - `cpt-bronze-to-api-e2e-seq-one-test-execution`

- **Use Cases Covered**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-usecase-author-test`

### 2.4 [Dbt Runner](feature-dbt-runner/) ⏳ HIGH

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-feature-dbt-runner`

- **Purpose**: Scoped, fast dbt execution per test. Owns manifest parsing (session-scoped), selector resolution, the deferred-state pattern, and per-worker schema injection via dbt vars.

- **Depends On**: `cpt-bronze-to-api-e2e-feature-test-rig-scaffolding`

- **Scope**:
  - Session-scoped `dbt parse` invocation populating `target/manifest.json`
  - Per-test `dbt build --select <spec.dbt_selector> --defer --state target/ --vars '{worker_id: {N}}'`
  - Surfacing failed-model details to pytest (dbt run-results parsing)
  - dbt version pin verification (warn if mismatch with prod)
  - Per-worker schema indirection wired through dbt vars consumed by existing models (this MAY require a follow-on patch to dbt models — explicitly tracked as risk)

- **Out of scope**:
  - Inserting into bronze (that's `ch-seeder` inside `csv-rig`)
  - Migration apply (`scaffolding`)

- **Requirements Covered**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-fr-dbt-run-scoped`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-nfr-per-test-latency`
  - [ ] `p2` - `cpt-bronze-to-api-e2e-fr-test-isolation`

- **Design Components**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-component-dbt-runner`

### 2.5 [API Spawner](feature-api-spawner/) ⏳ HIGH

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-feature-api-spawner`

- **Purpose**: Build `analytics-api` once per session with `cargo build --release`, spawn on `127.0.0.1:<random>`, expose a request-builder client. Owns the HTTP boundary.

- **Depends On**: `cpt-bronze-to-api-e2e-feature-test-rig-scaffolding`

- **Scope**:
  - `cargo build --release -p analytics-api` with `CARGO_TARGET_DIR` cached
  - Spawn with `INSIGHT_ANALYTICS_API_AUTH_DISABLED=true` and env vars pointing at the test ClickHouse + MariaDB
  - Random loopback port allocation
  - Startup wait against `GET /health`
  - Per-test request helper: builds the request from `spec.yaml`, POSTs, returns deserialized `ApiResponse`
  - Session teardown: SIGTERM, wait, SIGKILL on timeout

- **Out of scope**:
  - The `analytics-api` codebase itself (out of scope for this domain; lives in `src/backend/services/analytics-api`)
  - CSV assertion

- **Requirements Covered**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-fr-api-roundtrip`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-nfr-cold-start`

- **Design Components**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-component-api-client`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-constraint-loopback-only`

### 2.6 [ClickHouse Seeder & CSV Asserter](feature-csv-asserter/) ⏳ HIGH

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-feature-csv-asserter`

- **Purpose**: The two data-shaped components — typed CSV-into-bronze INSERT, and pandas-based cell-precise diff of API response vs expected CSV. Bundled into one feature because they share the same CSV-parsing helpers and Python type-coercion utilities.

- **Depends On**:

  - `cpt-bronze-to-api-e2e-feature-test-rig-scaffolding`
  - `cpt-bronze-to-api-e2e-feature-fixture-loader`

- **Scope**:
  - `ch-seeder`: type-aware CSV cell coercion via `system.columns`, batched INSERTs with `FORMAT CSVWithNames`, per-test touched-tables ledger, `truncate_touched()`
  - `csv-asserter`: response items flattening, column-set check, key-sorted diff, `assert_frame_equal` wrapper with cell-precise mismatch rendering
  - Float-tolerance handling (default `1e-6`, per-test override via `spec.yaml`)
  - First-20-mismatches output format

- **Out of scope**:
  - `--update-snapshots` mode (separate feature; this feature is pure-assert)
  - Anything that touches dbt or the API

- **Requirements Covered**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-fr-bronze-seed-from-csv`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-fr-bronze-truncate`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-fr-csv-assert`
  - [ ] `p2` - `cpt-bronze-to-api-e2e-nfr-diff-readability`

- **Design Components**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-component-ch-seeder`
  - [ ] `p1` - `cpt-bronze-to-api-e2e-component-csv-asserter`

### 2.7 [Snapshot Update](feature-snapshot-update/) ⏳ MEDIUM

- [ ] `p2` - **ID**: `cpt-bronze-to-api-e2e-feature-snapshot-update`

- **Purpose**: A `--update-snapshots` CLI flag that, instead of asserting, writes the actual response back to `expected/response.csv`. Used by developers to bootstrap a new test or acknowledge an intentional change. Refuses to run if `CI=true`.

- **Depends On**:

  - `cpt-bronze-to-api-e2e-feature-csv-asserter`
  - `cpt-bronze-to-api-e2e-feature-fixture-loader`

- **Scope**:
  - pytest CLI option registration
  - Path: when flag set, asserter writes actual back to disk instead of comparing
  - Refuse to run under `CI=true` (env check)
  - Logging: print which expected/*.csv files were updated and a git-style summary of cell changes

- **Out of scope**:
  - Anything that touches dbt, API spawn, or compose lifecycle

- **Requirements Covered**:

  - [ ] `p2` - `cpt-bronze-to-api-e2e-fr-csv-assert` (additive: update mode is an alternative branch of the same FR)

- **Design Principles Covered**:

  - [ ] `p2` - `cpt-bronze-to-api-e2e-principle-fixtures-are-truth`

- **Design Components**:

  - [ ] `p2` - `cpt-bronze-to-api-e2e-component-csv-asserter`

### 2.8 [CI Integration](feature-ci-integration/) ⏳ MEDIUM

- [ ] `p2` - **ID**: `cpt-bronze-to-api-e2e-feature-ci-integration`

- **Purpose**: Run the suite on every PR touching `src/ingestion/`, `src/backend/services/analytics-api/`, or `src/ingestion/scripts/migrations/`. Surface diffs in the job output.

- **Depends On**: `cpt-bronze-to-api-e2e-feature-csv-rig`

- **Scope**:
  - GitHub Actions workflow at `.github/workflows/e2e-bronze-to-api.yml`
  - Cargo target cache (`actions/cache`)
  - Docker image pre-warm step
  - `pytest -n auto` invocation
  - Failure annotation: surface cell-precise diff into the PR check
  - Path filter so the job only runs when relevant files change

- **Out of scope**:
  - Anything that runs locally only (lives in `csv-rig`)
  - Performance / load testing (out of PRD scope)

- **Requirements Covered**:

  - [ ] `p2` - `cpt-bronze-to-api-e2e-nfr-cold-start` (CI-side budget)
  - [ ] `p2` - `cpt-bronze-to-api-e2e-nfr-per-test-latency` (CI-side budget)

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-bronze-to-api-e2e-principle-shared-session`

- **Use Cases Covered**:

  - [ ] `p2` - `cpt-bronze-to-api-e2e-usecase-diagnose-failure`

---

## 3. Feature Dependencies

```text
cpt-bronze-to-api-e2e-feature-test-rig-scaffolding (foundation)
    │
    ├─→ cpt-bronze-to-api-e2e-feature-fixture-loader
    │       │
    │       ├─→ cpt-bronze-to-api-e2e-feature-csv-asserter ──┐
    │       │                                                 │
    │       └─→ cpt-bronze-to-api-e2e-feature-snapshot-update ┘ (also depends on csv-asserter)
    │
    ├─→ cpt-bronze-to-api-e2e-feature-dbt-runner ────────────┐
    │                                                         │
    └─→ cpt-bronze-to-api-e2e-feature-api-spawner ───────────┤
                                                              ▼
                                  cpt-bronze-to-api-e2e-feature-csv-rig (MVP)
                                                              │
                                                              ▼
                                  cpt-bronze-to-api-e2e-feature-ci-integration
```

**Dependency Rationale**:

- `fixture-loader`, `dbt-runner`, `api-spawner` all need the docker compose + pytest skeleton from `scaffolding`. They are independent of each other and can be developed in parallel after scaffolding ships.
- `csv-asserter` depends on `fixture-loader` because it consumes the expected DataFrame the loader produces; it does NOT depend on `dbt-runner` or `api-spawner` and can be unit-tested with a pre-built `ApiResponse`.
- `csv-rig` is the integration point — it consumes every prior feature. It MUST be last among the MVP set, because that's where the per-test orchestration lives.
- `snapshot-update` depends on `csv-asserter` (it's an alternate branch of the same component) and `fixture-loader` (to know where to write back). It does NOT block the MVP.
- `ci-integration` depends on `csv-rig` because there's nothing to run in CI until at least one full test passes.

**No circular dependencies** — verified by topological sort below.

---

## 4. Coverage Matrix

Every PRD FR/NFR is covered by ≥ 1 feature. Cells marked ✓ indicate primary ownership; cells marked (✓) indicate participation.

| Requirement / Component | scaffolding | fixture-loader | csv-rig | dbt-runner | api-spawner | csv-asserter | snapshot-update | ci-integration |
|---|---|---|---|---|---|---|---|---|
| **FR — bronze-seed-from-csv** | | (✓) | (✓) | | | ✓ | | |
| **FR — bronze-truncate** | | | (✓) | | | ✓ | | |
| **FR — dbt-run-scoped** | | | (✓) | ✓ | | | | |
| **FR — gold-view-queried** | ✓ | | (✓) | | | | | |
| **FR — api-roundtrip** | | | (✓) | | ✓ | | | |
| **FR — csv-assert** | | | (✓) | | | ✓ | (✓) | |
| **FR — test-isolation** | (✓) | | | ✓ | | | | |
| **NFR — cold-start** | ✓ | | | | (✓) | | | (✓) |
| **NFR — per-test-latency** | | | ✓ | (✓) | | | | (✓) |
| **NFR — parallel-safe** | (✓) | | | (✓) | | | | |
| **NFR — diff-readability** | | | | | | ✓ | (✓) | |
| **Component — fixture-loader** | | ✓ | | | | | | |
| **Component — ch-seeder** | | | | | | ✓ | | |
| **Component — dbt-runner** | | | | ✓ | | | | |
| **Component — migration-applier** | ✓ | | | | | | | |
| **Component — api-client** | | | | | ✓ | | | |
| **Component — csv-asserter** | | | | | | ✓ | (✓) | |
| **Component — session-rig** | ✓ | | (✓) | | | | | |
| **Sequence — session-startup** | ✓ | | | | | | | |
| **Sequence — one-test-execution** | | | ✓ | | | | | |
| **Use case — author-test** | | | ✓ | | | | | |
| **Use case — diagnose-failure** | | | | | | (✓) | | ✓ |

**Coverage check**:

- All 7 FRs covered: ✓
- All 4 NFRs covered: ✓
- All 7 components covered: ✓
- Both sequences covered: ✓
- Both use cases covered: ✓
- All 3 principles covered (across feature `Design Principles Covered` lists): `no-airbyte` → scaffolding; `shared-session` → scaffolding, csv-rig, ci-integration; `fixtures-are-truth` → fixture-loader, csv-rig, snapshot-update
- All 3 constraints covered: `version-parity` → scaffolding; `no-ddl-mutation` → scaffolding; `loopback-only` → scaffolding, api-spawner

---

## 5. Execution Order

| Phase | Features | Rationale |
|-------|----------|-----------|
| 1 | `feature-test-rig-scaffolding` | Foundation — nothing else runs without compose + pytest skeleton + migration-applier |
| 2 | `feature-fixture-loader`, `feature-dbt-runner`, `feature-api-spawner` | Independent of each other; can land in any order or in parallel PRs |
| 3 | `feature-csv-asserter` | Depends on `fixture-loader` (consumes expected DataFrame); independent of dbt-runner / api-spawner so can land in parallel with them |
| 4 | `feature-csv-rig` (MVP) | Integration point — consumes everything above. Includes one passing reference fixture against `insight.people`. **After this phase the framework is usable.** |
| 5 | `feature-snapshot-update` | Polish — developer experience for authoring new fixtures. Safe to land any time after `csv-asserter` |
| 6 | `feature-ci-integration` | Wire the framework into PR checks. Safe to land any time after `csv-rig`; runs in parallel with Phase 5 |

**Topological sort** (proves DAG has no cycles):

```text
1. feature-test-rig-scaffolding
2. feature-fixture-loader
3. feature-dbt-runner
4. feature-api-spawner
5. feature-csv-asserter
6. feature-csv-rig
7. feature-snapshot-update
8. feature-ci-integration
```
