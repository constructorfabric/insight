# Bronze-to-API E2E Test Framework

Test framework that exercises the full data path:

```
fixtures/<test>/bronze/*.csv  →  bronze tables  →  dbt staging/silver  →
ClickHouse migration gold-views  →  analytics-api HTTP  →  expected/response.csv
```

Airbyte / Kestra / Argo are NOT exercised — bronze is seeded by direct CSV INSERT.

See specs: [PRD](../../../../docs/domain/bronze-to-api-e2e/specs/PRD.md), [DESIGN](../../../../docs/domain/bronze-to-api-e2e/specs/DESIGN.md), [DECOMPOSITION](../../../../docs/domain/bronze-to-api-e2e/specs/DECOMPOSITION.md), [FEATURE csv-rig](../../../../docs/domain/bronze-to-api-e2e/specs/feature-csv-rig/FEATURE.md).

## Prerequisites

Only one: **Docker Engine ≥ 24**. Everything else (Python 3.12, Rust matching `rust-version` in `src/backend/Cargo.toml`, dbt-clickhouse, pytest, all deps) lives inside the runner image.

## Run (recommended — dockerized)

```bash
cd src/ingestion/tests/e2e

./e2e.sh build              # build the runner image (one-time, ~3-5 min cold)
./e2e.sh test               # full suite (includes people_smoke E2E)
./e2e.sh test -k people_smoke -v     # one fixture
./e2e.sh test -n auto       # ⚠️ parallel (pytest-xdist) — NOT supported yet: workers race on shared CH/MariaDB/dbt target
./e2e.sh shell              # interactive bash inside the runner
./e2e.sh down               # tear down compose stack + volumes
```

The same image (and the same `./e2e.sh test` invocation) is used in CI — see `.github/workflows/e2e-bronze-to-api.yml`.

First session bootstraps `cargo build --release -p analytics-api` (~3-5 min). Subsequent sessions reuse the named volume so cargo is incremental (~10s).

## Run (advanced — host-local)

If you prefer to develop on the host (faster iteration on the test code itself), install Python deps and rust on the host. The session-rig falls back to `E2E_RUN_MODE=host` which brings compose up via published ports on 127.0.0.1:30523/30506 (avoiding the in-cluster port-forwards).

```bash
python3.12 -m venv .venv
source .venv/bin/activate
pip install -e .
rustup update stable        # must satisfy rust-version in src/backend/Cargo.toml

pytest -k people_smoke -v   # session-rig brings compose up automatically
```

## Layout

```
e2e/
├── pyproject.toml              # deps; defines e2e_lib package
├── pytest.ini                  # pytest config
├── conftest.py                 # session-scoped pytest fixtures (the orchestrator)
├── compose/
│   ├── docker-compose.yml      # ClickHouse + MariaDB, loopback-only
│   └── .env.example            # example creds (real values generated per-session)
├── e2e_lib/                    # framework Python package
│   ├── compose.py              # docker compose up/down + healthcheck wait
│   ├── clickhouse.py           # CH HTTP client wrapper
│   ├── mariadb.py              # MariaDB connection helper
│   ├── migration_applier.py    # applies src/ingestion/scripts/migrations/*.sql
│   ├── analytics_api.py        # builds + spawns the analytics-api binary
│   ├── worker.py               # WorkerContext (resolves pytest-xdist worker id)
│   └── config.py               # session config (ports, random creds)
├── seed/
│   └── metrics.yaml            # optional test-specific metric overrides (default: empty)
├── fixtures/                   # individual fixture folders go here
└── meta/                       # framework's own smoke tests
    └── test_session_smoke.py
```

## Ports (loopback only)

| Service | Host port | Container port |
|---------|-----------|----------------|
| ClickHouse HTTP | `127.0.0.1:30523` | 8123 |
| ClickHouse native | `127.0.0.1:30529` | 9000 |
| MariaDB | `127.0.0.1:30506` | 3306 |
| analytics-api | `127.0.0.1:<random>` | — |

These ports avoid conflict with a local gitops dev cluster (which forwards 8123 / 3306) and the dbt local profile (30123).

## Notes for fixture authors

- Auth in `analytics-api` requires no Bearer token, but its tenant middleware rejects requests without a non-nil tenant. The harness sends `X-Insight-Tenant-Id` with `e2e_lib.config.TEST_TENANT_ID` on every request and re-homes seeded metric definitions onto that tenant (`metric_seed.py`). The ClickHouse query path does not filter by tenant yet, so bronze CSV rows may use any tenant value.
- Metric definitions are auto-seeded by the analytics-api binary's SeaORM migrations. Look up the metric UUID with `GET /v1/metrics` once the session is up, or add overrides in `seed/metrics.yaml`.
