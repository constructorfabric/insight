# Bronze-to-API E2E Test Framework

Test framework that exercises the full data path:

```
specs/<name>.test.yaml (bronze records)  →  bronze tables  →  dbt staging/silver  →
ClickHouse migration gold-views  →  analytics-api HTTP (POST /v1/metrics/queries)  →  expect rules
```

Airbyte / Kestra / Argo are NOT exercised — bronze is seeded by direct INSERT of the
`$ref`-resolved records declared in each `*.test.yaml`.

See specs: [PRD](../../../../docs/domain/bronze-to-api-e2e/specs/PRD.md), [DESIGN](../../../../docs/domain/bronze-to-api-e2e/specs/DESIGN.md), [DECOMPOSITION](../../../../docs/domain/bronze-to-api-e2e/specs/DECOMPOSITION.md), [FEATURE yaml-rig](../../../../docs/domain/bronze-to-api-e2e/specs/feature-yaml-rig/FEATURE.md).

## Data tier — attaches to the root stack

The e2e runner does **not** ship its own ClickHouse + MariaDB. It attaches to the data
tier defined in the repo-root [`docker-compose.yml`](../../../../docker-compose.yml) on the
`insight` network:

- If a dev stack is already up (`./dev-compose.sh up`), the runner **reuses** its
  `clickhouse` + `mariadb` by service name.
- If not, the runner's `depends_on` **brings up** just those two services (never the
  backend/frontend services).

Credentials and ports come from a committed, test-specific env file
[`compose/e2e.env`](compose/e2e.env) (not a developer's personal `.env.compose`). Its
values match the root compose defaults (`insight` / `insight-local`, ports `8123/9000/3306`),
so the runner attaches cleanly to a default `./dev-compose.sh up` and to a fresh CI bring-up
alike. If your dev stack uses custom credentials/ports, point the harness at your own file:
`E2E_ENV_FILE=../../../../.env.compose ./e2e.sh test`. The runner still **builds + spawns
its own `analytics-api` from current source** — it never uses the root stack's
`analytics-api` container.

> Attach mode is destructive to whatever DB you point at: the harness seeds
> bronze/silver/gold + metric definitions into the root stack's `insight` (CH) and
> `analytics` (MariaDB) databases. `./e2e.sh down` removes **only the runner** — the data
> tier is the root stack's to manage (`./dev-compose.sh down`).

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
./e2e.sh down               # remove the runner (data tier left running — see above)
```

The same image (and the same `./e2e.sh test` invocation) is used in CI — see `.github/workflows/e2e-bronze-to-api.yml`.

First session bootstraps `cargo build --release -p analytics-api` (~3-5 min). Subsequent sessions reuse the named volume so cargo is incremental (~10s).

## Run (advanced — host-local)

If you prefer to develop on the host (faster iteration on the test code itself), install Python deps and rust on the host. The session-rig falls back to `E2E_RUN_MODE=host`, which brings up the root stack's `clickhouse` + `mariadb` and connects via their published loopback ports (`127.0.0.1:8123` / `127.0.0.1:3306`). If your root stack uses non-default credentials, export the matching `E2E_CH_PASSWORD` / `E2E_MARIADB_PASSWORD`.

```bash
python3.12 -m venv .venv
source .venv/bin/activate
pip install -e .
rustup update stable        # must satisfy rust-version in src/backend/Cargo.toml

pytest -k collab_emails_sent -v   # session-rig brings the data tier up automatically
```

## Layout

```
e2e/
├── pyproject.toml              # deps; defines e2e_lib package
├── pytest.ini                  # pytest config
├── conftest.py                 # session-scoped pytest fixtures (the orchestrator)
├── compose/
│   ├── docker-compose.runner.yml  # adds ONLY the runner; layered on repo-root docker-compose.yml
│   ├── e2e.env                    # committed test-specific env (creds/ports for the data tier)
│   └── Dockerfile.runner          # runner image (python+rust+deps)
├── e2e_lib/                    # framework Python package
│   ├── compose.py              # brings up the root data tier (host mode) + healthcheck wait
│   ├── clickhouse.py           # CH HTTP client wrapper
│   ├── mariadb.py              # MariaDB connection helper
│   ├── migration_applier.py    # applies src/ingestion/scripts/migrations/*.sql
│   ├── analytics_api.py        # builds + spawns the analytics-api binary
│   ├── worker.py               # WorkerContext (resolves pytest-xdist worker id)
│   └── config.py               # session config (ports, random creds)
├── seed/
│   └── metrics.yaml            # optional test-specific metric overrides (default: empty)
├── specs/                      # <name>.test.yaml + schemas/ + templates/
└── meta/                       # framework's own smoke tests
    └── test_session_smoke.py
```

## Ports

In **docker mode** (the default `./e2e.sh test` path) the runner reaches the data tier
in-network by service name — no host ports are involved:

| Service | In-network endpoint |
|---------|---------------------|
| ClickHouse HTTP | `clickhouse:8123` |
| ClickHouse native | `clickhouse:9000` |
| MariaDB | `mariadb:3306` |
| analytics-api | `127.0.0.1:<random>` (inside the runner) |

In **host mode** (host-local pytest) the harness connects to the root stack's published
loopback ports — `127.0.0.1:8123` / `9000` / `3306` by default (override with the root
compose's `CLICKHOUSE_HTTP_PORT` / `CLICKHOUSE_NATIVE_PORT` / `MARIADB_PORT`). If you also
run a local gitops dev cluster that forwards 8123/3306, stop one of the two — they share
the same host ports now that e2e rides the root stack.

## Notes for fixture authors

- Auth in `analytics-api` requires no Bearer token, but its tenant middleware rejects requests without a non-nil tenant. The harness sends `X-Insight-Tenant-Id` with `e2e_lib.config.TEST_TENANT_ID` on every request and re-homes seeded metric definitions onto that tenant (`metric_seed.py`). The ClickHouse query path does not filter by tenant yet, so seeded bronze rows may use any tenant value.
- Metric definitions are auto-seeded by the analytics-api binary's SeaORM migrations. Look up the metric UUID with `GET /v1/metrics` once the session is up, or add overrides in `seed/metrics.yaml`.

## `cases` / `expect` (declarative YAML rig)

Tests are `specs/**/*.test.yaml`; each `case` POSTs a batch to `/v1/metrics/queries` and checks an `expect` list of rules. A rule selects with `in` (batch result by `id`) + an exact-equality `find` (`{field: value}`), then asserts via `equal` (subset of fields, exact / `null`) or `assert` (a CEL boolean). Anything richer than equality (inequalities, counts, predicates) goes in a CEL `assert` — the rig deliberately has no second selector language. See the [yaml-rig FEATURE](../../../../docs/domain/bronze-to-api-e2e/specs/feature-yaml-rig/FEATURE.md) and the `/metric-e2e-test` skill.

Variables available in an `assert` (CEL) expression — assembled in `e2e_lib/expect_engine.py::evaluate_case` (the `bindings` dict), converted to CEL in `_eval_cel`:

| Binding | Value | Present when |
|---------|-------|--------------|
| `it` | the single row matched by `find` | only with `find` (else `null`) |
| `items` | the selected result's `items` array | a result is selected (`in` or sole query) |
| `result` | the selected batch result `{id, status, metric_id, items, page_info}` | a result is selected |
| `results` | the full `results[]` of the batch | always |
| `status` | the batch HTTP status code (int) | always |

CEL is strictly typed and will not compare an `int` to a `double`. Bindings are passed through unchanged, so when a metric value may be integral (e.g. `40`) and you compare against a fractional literal, cast it: `double(it.value) > 39.5`. `status` and `size(...)` are integers — compare them with integer literals. Use `equal` for exact / `null` comparisons (it uses Python `==`).

### What is CEL

`assert` expressions are written in **CEL — the [Common Expression Language](https://github.com/google/cel-spec)** (the same expression language used by Kubernetes admission policies and Envoy). It is a small, side-effect-free language for boolean/value expressions over structured data: no statements, no loops, no I/O — an expression is evaluated against the bindings above and must return a boolean. The rig evaluates it with the [`cel-python`](https://pypi.org/project/cel-python/) library (`celpy`) in `e2e_lib/expect_engine.py::_eval_cel`.

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
