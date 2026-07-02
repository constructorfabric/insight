# Bronze-to-API E2E Test Framework

Test framework that exercises the full data path:

```
metrics/<name>.test.yaml (bronze records)  →  bronze tables  →  dbt staging/silver  →
ClickHouse migration gold-views  →  analytics-api HTTP (POST /v1/metrics/queries)  →  expect rules
```

Airbyte / Kestra / Argo are NOT exercised — bronze is seeded by direct INSERT of the
`$ref`-resolved records declared in each `*.test.yaml`.

### Execution model — migrate once, seed once, assert many

`docker compose` brings the database stack up and applies **all** migrations *before*
pytest runs; the fixture set is then materialised **once per session**, and each test
is a pure assertion against that shared world:

```
compose (once):  up clickhouse + mariadb  →  e2e-migrate runs the REAL
                 src/ingestion/scripts/apply-ch-migrations.sh
                 (core DBs, bronze placeholders, gold views)
build   (once):  seed EVERY fixture's bronze  →  dbt build staging → enrich → dbt build silver
                 →  reapply gold-view migrations (rebind to real silver)  →  refresh MVs
per test:        namespace the case's request  →  POST /v1/metrics/queries  →  evaluate expects
```

The stack reuses the ROOT `docker-compose.yml` clickhouse + mariadb (single version
SSOT — the versions the gitops charts deploy), run as an **isolated `insight-e2e`
compose project** with its own volumes, so a running `./dev-compose.sh up` dev stack
is left untouched. The overlay that adds the migrate + runner steps is
`/docker-compose.e2e.yml`.

To let all fixtures coexist in one ClickHouse, each fixture's identity is rewritten to a
private namespace (`lib/namespace.py`): the email domain, plus `unique_key` / `id` /
`source_id` / `department`. That keeps every ReplacingMergeTree ORDER-BY key unique (no
cross-fixture collapse) and every query scoped to its own data. `meta/test_seed_isolation.py`
proves the invariant offline by parsing each table's real ORDER BY key from the placeholder
DDL. Because the build is session-scoped, `-k <name>` still materialises the **whole** world
(the filter only selects which assertions run), so a single-test run is not faster to seed.

See specs: [PRD](../../../../docs/domain/bronze-to-api-e2e/specs/PRD.md), [DESIGN](../../../../docs/domain/bronze-to-api-e2e/specs/DESIGN.md), [DECOMPOSITION](../../../../docs/domain/bronze-to-api-e2e/specs/DECOMPOSITION.md), [FEATURE yaml-rig](../../../../docs/domain/bronze-to-api-e2e/specs/feature-yaml-rig/FEATURE.md).

## Prerequisites

Only one: **Docker Engine ≥ 24**. Everything else (Python 3.12, Rust matching `rust-version` in `src/backend/Cargo.toml`, dbt-clickhouse, pytest, all deps) lives inside the runner image.

## Run (recommended — dockerized)

```bash
cd src/ingestion/tests/e2e

./e2e.sh build              # build the runner image (one-time, ~3-5 min cold)
./e2e.sh test               # full suite
./e2e.sh test -k collab_emails_sent -v   # one test
./e2e.sh test -n auto       # ⚠️ parallel (pytest-xdist) — NOT supported yet: workers race on shared CH/MariaDB/dbt target
./e2e.sh shell              # interactive bash inside the runner
./e2e.sh down               # tear down compose stack + volumes
```

The same image (and the same `./e2e.sh test` invocation) is used in CI — see `.github/workflows/e2e-bronze-to-api.yml`.

First session bootstraps `cargo build --release -p analytics-api` (~3-5 min). Subsequent sessions reuse the named volume so cargo is incremental (~10s).

## Run (advanced — host-local)

If you prefer to develop on the host (faster iteration on the test code itself), install Python deps and rust on the host. Bring the stack up + migrated once with `./e2e.sh up` (publishes CH/MariaDB on 127.0.0.1:30523/30506), then run pytest directly — it connects in `E2E_RUN_MODE=host` (the default) and does NOT manage compose or migrations.

```bash
python3.12 -m venv .venv
source .venv/bin/activate
pip install -e .
rustup update stable        # must satisfy rust-version in src/backend/Cargo.toml
cargo build --release -p analytics-api   # host-mode: the rig spawns target/release/analytics-api

./e2e.sh up                       # start DBs + apply migrations (once)
pytest -k collab_emails_sent -v   # connects to the already-up, migrated stack
```

## Layout

The compose topology lives at the **repo root** (shared with the dev stack):
`docker-compose.yml` (clickhouse + mariadb — version SSOT), `docker-compose.e2e.yml`
(the e2e overlay: isolation + `e2e-migrate` + `e2e-runner`), and
`docker-compose.e2e.cache.yml` (CI-only BuildKit cache). The `e2e/` tree itself:

```
e2e/
├── pyproject.toml              # deps; defines lib package
├── pytest.ini                  # pytest config
├── e2e.sh                      # entrypoint: drives the root compose as project insight-e2e
├── conftest.py                 # session-scoped pytest fixtures (the orchestrator)
├── compose/
│   └── Dockerfile.runner       # runner image (python+dbt; bakes analytics-api + enrich bins)
├── lib/                        # framework Python package
│   ├── clickhouse.py           # CH HTTP client wrapper
│   ├── mariadb.py              # MariaDB connection helper
│   ├── migration_applier.py    # post-dbt gold-view reapply + refreshable-MV refresh
│   ├── namespace.py            # per-fixture identity namespacing (seed-once isolation)
│   ├── seed_once.py            # one-shot world builder (reset → seed all → dbt → rebind → refresh)
│   ├── analytics_api.py        # locates + spawns the baked analytics-api binary
│   ├── worker.py               # WorkerContext (resolves pytest-xdist worker id)
│   ├── metric_coverage.py      # metric-coverage gate: SKIP_TABLES + SKIP_LIST (--universe-file)
│   ├── collect_metrics.py      # script: snapshot the metric catalog → .artifacts/
│   └── config.py               # session config (ports, random creds)
├── seed/
│   └── metrics.yaml            # optional test-specific metric overrides (default: empty)
├── metrics/                    # <name>.test.yaml + schemas/ + templates/
├── api/                        # rig smoke tests (session fixtures + analytics-api health)
│   └── test_session_smoke.py
└── meta/                       # DB-free guard
    └── test_seed_isolation.py
```

## Metric coverage gate

A job (`metric-coverage-gate`) in the **E2E — Bronze to API** workflow, *not* a pytest test. The `e2e` job runs the suite and, while analytics-api is up, snapshots the metric catalog (`POST /v1/catalog/get_metrics`) to `.artifacts/catalog_metrics.json` (uploaded as `coverage-inputs`); the gate job then checks every product `metric_key` the catalog exposes is value-asserted by a test or covered by a `SKIP_TABLES`/`SKIP_LIST` entry — pure Python, no Docker, no second app boot.

Locally, after a run:

```bash
./e2e.sh test     # runs the suite + snapshots .artifacts/catalog_metrics.json
./e2e.sh gates    # runs the metric-coverage gate against it (in the runner image; no DB)
```

The verdict per **metric_key** (each individual number) is **binary**:

- **value-tested** — a `metrics/*.test.yaml` asserts it (`find: {metric_key: …}` paired with `equal`/`assert`) → **PASS**
- **skip-listed** (in the inline `SKIP_LIST` in [`lib/metric_coverage.py`](lib/metric_coverage.py)) → **PASS** (baseline)
- **neither** → **FAIL** — a number nobody validates must get an assertion or a `SKIP_LIST` entry.

Catalog keys are dotted (`collab_bullet_rows.m365_emails_sent`); a test asserts the bare response key (`m365_emails_sent`). The column suffix is unique across the catalog, so the gate maps bare→dotted by suffix (a future collision raises). `SKIP_LIST` is the accepted baseline and single source of truth (no side-car file — just `(metric_key, reason)`). Kept honest: a **stale** entry (key no longer in the catalog), a **redundant** one (now value-tested), or a test asserting a **non-catalog** key (typo / unseeded → matches 0 rows) all fail. PASS iff no FAILs.

```bash
# ad hoc against a running analytics-api (instead of the collected artifact):
ANALYTICS_API_URL=http://localhost:18081 python3 lib/metric_coverage.py
```

Coverage is **per metric_key**, so every number on a bullet is validated independently — one tested key of a metric does not cover the rest. Today: **44/96** value-tested; the rest are skip-listed with a reason (`reachable — …` entries are the backlog where fixtures already exist).

## Ports (loopback only)

| Service | Host port | Container port |
|---------|-----------|----------------|
| ClickHouse HTTP | `127.0.0.1:30523` | 8123 |
| ClickHouse native | `127.0.0.1:30529` | 9000 |
| MariaDB | `127.0.0.1:30506` | 3306 |
| analytics-api | `127.0.0.1:<random>` | — |

These ports avoid conflict with a local gitops dev cluster (which forwards 8123 / 3306) and the dbt local profile (30123).

## Notes for fixture authors

- Auth in `analytics-api` requires no Bearer token, but its tenant middleware rejects requests without a non-nil tenant. The harness sends `X-Insight-Tenant-Id` with `lib.config.TEST_TENANT_ID` on every request and re-homes seeded metric definitions onto that tenant (`metric_seed.py`). The ClickHouse query path does not filter by tenant yet, so seeded bronze rows may use any tenant value.
- Metric definitions are auto-seeded by the analytics-api binary's SeaORM migrations. Look up the metric UUID with `GET /v1/metrics` once the session is up, or add overrides in `seed/metrics.yaml`.

## `cases` / `expect` (declarative YAML rig)

Tests are `metrics/**/*.test.yaml`; each `case` POSTs a batch to `/v1/metrics/queries` and checks an `expect` list of rules. A rule selects with `in` (batch result by `id`) + an exact-equality `find` (`{field: value}`), then asserts via `equal` (subset of fields, exact / `null`) or `assert` (a CEL boolean). Anything richer than equality (inequalities, counts, predicates) goes in a CEL `assert` — the rig deliberately has no second selector language. See the [yaml-rig FEATURE](../../../../docs/domain/bronze-to-api-e2e/specs/feature-yaml-rig/FEATURE.md) and the `/metric` skill.

Variables available in an `assert` (CEL) expression — assembled in `lib/expect_engine.py::evaluate_case` (the `bindings` dict), converted to CEL in `_eval_cel`:

| Binding | Value | Present when |
|---------|-------|--------------|
| `it` | the single row matched by `find` | only with `find` (else `null`) |
| `items` | the selected result's `items` array | a result is selected (`in` or sole query) |
| `result` | the selected batch result `{id, status, metric_id, items, page_info}` | a result is selected |
| `results` | the full `results[]` of the batch | always |
| `status` | the batch HTTP status code (int) | always |

CEL is strictly typed and will not compare an `int` to a `double`. Bindings are passed through unchanged, so when a metric value may be integral (e.g. `40`) and you compare against a fractional literal, cast it: `double(it.value) > 39.5`. `status` and `size(...)` are integers — compare them with integer literals. Use `equal` for exact / `null` comparisons (it uses Python `==`).

### What is CEL

`assert` expressions are written in **CEL — the [Common Expression Language](https://github.com/google/cel-spec)** (the same expression language used by Kubernetes admission policies and Envoy). It is a small, side-effect-free language for boolean/value expressions over structured data: no statements, no loops, no I/O — an expression is evaluated against the bindings above and must return a boolean. The rig evaluates it with the [`cel-python`](https://pypi.org/project/cel-python/) library (`celpy`) in `lib/expect_engine.py::_eval_cel`.

Operators: `== != < <= > >=`, `&& || !`, `+ - * / %`, `in`, ternary `cond ? a : b`. Field/index access: `it.value`, `result.status`, `items[0]`. Useful built-ins & macros: `size(x)`, `has(x.field)`, `x.exists(e, <pred>)`, `x.all(e, <pred>)`, `x.filter(e, <pred>)`, `x.map(e, <expr>)`, string `.startsWith()/.endsWith()/.contains()/.matches(re)`.

Examples:

```yaml
- assert: "status == 200"                                  # batch HTTP code
- in: collaboration
  assert: "result.status == 'ok'"                           # this query's own status
- in: collaboration
  assert: "size(items) == 20"                               # row count
- in: collaboration
  find: { metric_key: m365_emails_sent }
  assert: "double(it.value) > 39.5 && double(it.value) < 40.5"   # cast to double for fractional compare
- in: collaboration
  find: { metric_key: slack_dm_ratio }
  assert: "it.value == null"                                # explicit null
- assert: "results.exists(r, r.status == 'error')"          # any query in the batch failed?
- in: collaboration
  assert: "items.all(r, r.range_min <= r.value)"            # invariant across all rows
```

Prefer `equal` for exact / `null` checks (it uses Python `==`, so `40 == 40.0` and `value: null` work directly); reach for `assert` when you need inequalities, counts, or cross-row predicates.
