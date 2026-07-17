# Bronze-to-API E2E Test Framework


<!-- toc -->

- [Prerequisites](#prerequisites)
- [Run (recommended — dockerized)](#run-recommended--dockerized)
- [Run (advanced — host-local)](#run-advanced--host-local)
- [Layout](#layout)
- [Metric coverage gate](#metric-coverage-gate)
- [API endpoint coverage gate](#api-endpoint-coverage-gate)
- [Ports (loopback only)](#ports-loopback-only)
- [Notes for fixture authors](#notes-for-fixture-authors)
- [`cases` / `expect` (declarative YAML rig)](#cases--expect-declarative-yaml-rig)
  - [What is CEL](#what-is-cel)

<!-- /toc -->

Test framework that exercises the full data path:

```
metrics/<name>.test.yaml (bronze records)  →  bronze tables  →  dbt staging/silver/gold  →
analytics HTTP (POST /v1/metric-results)  →  expect rules
```

Airbyte / Kestra / Argo are NOT exercised — bronze is seeded by direct INSERT of the
`$ref`-resolved records declared in each `*.test.yaml`.

See specs: [PRD](../../../../docs/domain/bronze-to-api-e2e/specs/PRD.md), [DESIGN](../../../../docs/domain/bronze-to-api-e2e/specs/DESIGN.md), [DECOMPOSITION](../../../../docs/domain/bronze-to-api-e2e/specs/DECOMPOSITION.md), [FEATURE yaml-rig](../../../../docs/domain/bronze-to-api-e2e/specs/feature-yaml-rig/FEATURE.md).

## Prerequisites

Only one: **Docker Engine ≥ 24**. Everything else (Python 3.12, Rust matching `rust-version` in `src/backend/Cargo.toml`, dbt-clickhouse, pytest, all deps) lives inside the runner image.

## Run (recommended — dockerized)

```bash
cd src/ingestion/tests/e2e

./e2e.sh build              # build the runner image (one-time, ~3-5 min cold)
./e2e.sh test               # full suite (api + metrics + meta)
./e2e.sh test api/          # api suite: endpoint contract tests only (seconds)
./e2e.sh test metrics/      # metrics suite: the yaml fixture rig
./e2e.sh test meta/         # rig framework self-tests (local-only; CI skips them)
./e2e.sh test -k collab_emails_sent -v   # one test
./e2e.sh test -n auto       # ⚠️ parallel (pytest-xdist) — NOT supported yet: workers race on shared CH/MariaDB/dbt target
./e2e.sh shell              # interactive bash inside the runner
./e2e.sh down               # tear down compose stack + volumes
```

The same image is used in CI, which builds it ONCE in a shared upstream `build` job and hands it to two independent lanes (`e2e-api`, `e2e-metrics`) as a saved image artifact — each lane loads the image, boots its own stack, runs its own suite (meta/ is local-only: it tests the harness, not the product), and uploads its own coverage artifact (`coverage-inputs-api` / `coverage-inputs-metrics`) — see `.github/workflows/e2e-bronze-to-api.yml`. The lanes share nothing at runtime (only the build is shared); each gate consumes only its own lane's artifact.

First session bootstraps `cargo build --release -p analytics` (~3-5 min). Subsequent sessions reuse the named volume so cargo is incremental (~10s).

## Run (advanced — host-local)

If you prefer to develop on the host (faster iteration on the test code itself), install Python deps and rust on the host. The session-rig falls back to `E2E_RUN_MODE=host` which brings compose up via published ports on 127.0.0.1:30523/30506 (avoiding the in-cluster port-forwards).

```bash
python3.12 -m venv .venv
source .venv/bin/activate
pip install -e .
rustup update stable        # must satisfy rust-version in src/backend/Cargo.toml

pytest -k collab_emails_sent -v   # session-rig brings compose up automatically
```

## Layout

```
e2e/
├── pyproject.toml              # deps; defines lib package
├── pytest.ini                  # pytest config
├── conftest.py                 # session-scoped pytest fixtures (the orchestrator)
├── compose/
│   ├── docker-compose.yml      # ClickHouse + MariaDB, loopback-only
│   └── .env.example            # example creds (real values generated per-session)
├── lib/                        # framework Python package
│   ├── compose.py              # docker compose up/down + healthcheck wait
│   ├── clickhouse.py           # CH HTTP client wrapper
│   ├── mariadb.py              # MariaDB connection helper
│   ├── migration_applier.py    # applies src/ingestion/scripts/migrations/*.sql
│   ├── analytics.py        # builds + spawns the analytics binary
│   ├── worker.py               # WorkerContext (resolves pytest-xdist worker id)
│   ├── metric_coverage.py      # builtin metric coverage gate
│   ├── api_coverage.py         # endpoint-coverage report + httpx recording hook
│   ├── collect_metric_definitions.py # snapshot enabled builtin definitions
│   └── config.py               # session config (ports, random creds)
├── seed/
│   └── metrics.yaml            # optional test-specific metric overrides (default: empty)
├── metrics/                      # <name>.test.yaml + schemas/ + templates/
└── meta/                       # the rig's own framework tests (dbt runner, expect engine, ref resolver)
```

## Metric coverage gate

A job (`metric-coverage-gate`) in the **E2E — Bronze to API** workflow checks enabled builtin product definitions. The metrics lane snapshots `metric_key`, computation, dimensions, and cohort metadata to `.artifacts/metric_definitions.json`; the gate then verifies every builtin has period, peer, and timeseries assertions, plus breakdown assertions for declared dimensions and histogram assertions for median metrics.

Locally, after a run:

```bash
./e2e.sh test metrics/    # runs the metrics suite and writes metric_definitions.json
./e2e.sh gates metrics    # metric gate only, against .artifacts/ (in the runner image; no DB)
```

`./e2e.sh gates` with no argument runs both gates (handy after running both suites locally; see [API endpoint coverage gate](#api-endpoint-coverage-gate) below for the api/-only equivalent). `gates api` / `gates metrics` run one gate against one artifact each — that per-lane shape is what mirrors the two independent CI jobs, each of which only ever needs its own lane's artifact.

Missing views, unknown asserted keys, unknown requested keys, and stale fixtures fail. There are no legacy suffix mappings or skip lists.

```bash
# ad hoc against a running analytics (instead of the collected artifact):
ANALYTICS_URL=http://localhost:18081 python3 lib/metric_coverage.py
```

Coverage is **per metric_key**, so every number on a bullet is validated independently — one tested key of a metric does not cover the rest. Today: **44/96** value-tested; the rest are skip-listed with a reason (`reachable — …` entries are the backlog where fixtures already exist).

## API endpoint coverage gate

The suite records which analytics routes it exercises: an httpx response hook on the rig's single client chokepoint (`AnalyticsProcess.client()`) accumulates `(method, path) → {status codes}`, and `conftest.pytest_sessionfinish` dumps the ledger to `.artifacts/observed_endpoints.json` (shipped to CI inside the `coverage-inputs-api` artifact, produced by the `e2e-api` lane). The `api-endpoint-coverage-gate` job diffs it against the committed OpenAPI spec (`docs/components/backend/analytics/openapi.json` — kept accurate by the analytics OpenAPI drift gate: the `openapi_spec_matches_committed` golden test + the `openapi-specs` workflow) via [`lib/api_coverage.py`](lib/api_coverage.py). **Blocking is at the operation level**: the gate fails only when a documented operation is exercised by NO test — a new endpoint added to the spec without a contract test — or when a `SKIP_LIST` entry rots (now exercised, or gone from the spec). It does **not** fail on an individual unobserved status code.

Locally, after a run:

```bash
./e2e.sh test api/    # runs the api/ suite (emits both .artifacts files; only observed_endpoints.json feeds this gate)
./e2e.sh gates api    # endpoint gate only, against .artifacts/ (in the runner image; no DB)
```

Per-status-code coverage is **reported, not enforced**: the report renders an endpoints × registered-status-codes table (`✓` observed · `✗` declared but not yet observed · `·` excluded · blank = not declared) and an overall coverage percentage. A code is *coverable* — and so counts toward the percentage — only if a black-box rig can produce it: `coverable(op) = declared(op) − {codes ≥ 500} − UNIVERSAL_BOILERPLATE{401,429} − BLOCKED[op]`. `BLOCKED` absorbs the committed spec's `.standard_errors` over-declaration (#1669) plus pinned rig/product limits, and a `·` code that becomes observed (or a `BLOCKED` op dropped from the spec) is surfaced as a non-blocking advisory so the list stays honest.

The [`api/`](api/) contract suite covers all 21 spec operations — one module per path group (`test_metrics.py`, `test_metric_thresholds.py`, `test_admin_thresholds.py`, `test_catalog.py`, `test_columns.py`, `test_persons.py`, `test_metric_results.py`), one test per (path, method, status-code) case, from self-cleaning fixtures (`api/conftest.py`). The declarative metrics suite covers successful `/v1/metric-results` computation for every builtin metric.

## Ports (loopback only)

| Service | Host port | Container port |
|---------|-----------|----------------|
| ClickHouse HTTP | `127.0.0.1:30523` | 8123 |
| ClickHouse native | `127.0.0.1:30529` | 9000 |
| MariaDB | `127.0.0.1:30506` | 3306 |
| analytics | `127.0.0.1:<random>` | — |

These ports avoid conflict with a local gitops dev cluster (which forwards 8123 / 3306) and the dbt local profile (30123).

## Notes for fixture authors

- Auth in `analytics` requires no Bearer token, but its tenant middleware rejects requests without a non-nil tenant. The harness sends `X-Insight-Tenant-Id` with `lib.config.TEST_TENANT_ID` on every request and re-homes seeded metric definitions onto that tenant (`metric_seed.py`). The ClickHouse query path does not filter by tenant yet, so seeded bronze rows may use any tenant value.
- Metric definitions are auto-seeded by the analytics binary's SeaORM migrations. Look up the metric UUID with `GET /v1/metrics` once the session is up, or add overrides in `seed/metrics.yaml`.

## `cases` / `expect` (declarative YAML rig)

Tests are `metrics/**/*.test.yaml`; each case posts to `/v1/metric-results`. Rules select a full builtin `metric` key and `view`, then optionally use `find` for a row and `equal` for exact or null values. Use CEL `assert` for nested timeseries points, dimensions, histogram bins, counts, and predicates.

Variables available in an `assert` (CEL) expression — assembled in `lib/expect_engine.py::evaluate_case` (the `bindings` dict), converted to CEL in `_eval_cel`:

| Binding | Value | Present when |
|---------|-------|--------------|
| `it` | the single row matched by `find` | only with `find` (else `null`) |
| `items` | values or series from the selected view | a metric view is selected |
| `view` | the selected view object | a metric view is selected |
| `metric` | the selected metric result | a metric is selected |
| `metrics` | all metric results | always |
| `status` | the batch HTTP status code (int) | always |

CEL is strictly typed and will not compare an `int` to a `double`. Bindings are passed through unchanged, so when a metric value may be integral (e.g. `40`) and you compare against a fractional literal, cast it: `double(it.value) > 39.5`. `status` and `size(...)` are integers — compare them with integer literals. Use `equal` for exact / `null` comparisons (it uses Python `==`).

### What is CEL

`assert` expressions are written in **CEL — the [Common Expression Language](https://github.com/google/cel-spec)** (the same expression language used by Kubernetes admission policies and Envoy). It is a small, side-effect-free language for boolean/value expressions over structured data: no statements, no loops, no I/O — an expression is evaluated against the bindings above and must return a boolean. The rig evaluates it with the [`cel-python`](https://pypi.org/project/cel-python/) library (`celpy`) in `lib/expect_engine.py::_eval_cel`.

Operators: `== != < <= > >=`, `&& || !`, `+ - * / %`, `in`, ternary `cond ? a : b`. Field/index access: `it.value`, `view.values`, `items[0]`. Useful built-ins & macros: `size(x)`, `has(x.field)`, `x.exists(e, <pred>)`, `x.all(e, <pred>)`, `x.filter(e, <pred>)`, `x.map(e, <expr>)`, string `.startsWith()/.endsWith()/.contains()/.matches(re)`.

Examples:

```yaml
- assert: "status == 200"
- metric: collab.emails_sent
  view: period
  find: { entity_id: erin@example.com }
  equal: { value: 50 }
- metric: collab.messages_sent
  view: timeseries
  assert: "items.exists(s, s.points.exists(p, double(p.value) > 0.0))"
- metric: git.lines_added
  view: breakdown
  assert: "items.exists(v, v.dimensions.exists(d, d.key == 'category' && d.value == 'code'))"
```

Prefer `equal` for exact / `null` checks (it uses Python `==`, so `40 == 40.0` and `value: null` work directly); reach for `assert` when you need inequalities, counts, or cross-row predicates.
