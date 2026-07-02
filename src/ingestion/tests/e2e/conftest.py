"""Session orchestrator — the central pytest conftest.

SEED-ONCE model, EXTERNALLY-MANAGED infra. The database stack is brought up AND
migrated BEFORE pytest runs — `docker compose` starts ClickHouse + MariaDB and
the `e2e-migrate` service applies every ClickHouse migration (see ../e2e.sh and
/docker-compose.e2e.yml). pytest connects to that already-migrated stack; it does
NOT boot compose or apply base migrations. It then:

  session build (once, `build_world` fixture — see lib/seed_once.py):
    1. reset the multi-reader incremental tables (warm-rerun determinism)
    2. seed EVERY fixture's namespaced bronze in one shot
    3. dbt build staging (union) -> connector enrich -> dbt build silver (union)
    4. reapply gold-view migrations ONCE  (rebind views to the real silver)
    5. refresh refreshable MVs ONCE
    6. (analytics_api fixture) spawn analytics-api + seed metric definitions

  per test (`test_metric_smoke`):
    namespace the case's request -> POST /v1/metrics/queries -> evaluate expects.
    No seeding, no dbt, no migration — the data is already there.

Isolation between fixtures comes from `lib.namespace`; the guard
`meta/test_seed_isolation.py` proves no two fixtures collide. The suite is serial
(not xdist-safe: build_world and the shared analytics-api process are
single-owner).
"""

from __future__ import annotations

import logging
import os
import time

from pathlib import Path

import pytest

from lib import clickhouse as ch
from lib import mariadb, seed_once
from lib.analytics_api import AnalyticsApiProcess, find_free_port, locate_binary
from lib.ch_seeder import CHSeeder
from lib.config import SessionConfig, TEST_TENANT_ID
from lib.dbt_runner import DbtRunner
from lib.enrich import EnrichRunner
from lib.fixture_loader import TestYaml, discover_tests, load as load_test
from lib.metric_seed import seed_test_metrics
from lib.worker import WorkerContext

LOG = logging.getLogger("e2e.rig")


# ----------------------------------------------------------------------
# Worker-aware session lifecycle
# ----------------------------------------------------------------------
#
# The suite is serial (not xdist-safe yet: the build_world step and the shared
# analytics-api process are single-owner). We still elect a primary worker so a
# future xdist run doesn't double-seed the shared stack.

_IS_XDIST = bool(os.environ.get("PYTEST_XDIST_WORKER"))
_IS_PRIMARY = not _IS_XDIST or os.environ.get("PYTEST_XDIST_WORKER") == "gw0"

_METRICS_ROOT = Path(__file__).parent / "metrics"


# ----------------------------------------------------------------------
# Fixtures
# ----------------------------------------------------------------------


@pytest.fixture(scope="session")
def session_cfg() -> SessionConfig:
    """Resolve session config once."""
    cfg = SessionConfig.from_env()
    LOG.info("session config: ch=%s, mariadb=%s", cfg.ch_http_url, cfg.mariadb_dsn)
    return cfg


@pytest.fixture(scope="session")
def worker_ctx() -> WorkerContext:
    return WorkerContext.from_env()


def _wait_ch_ready(cfg: SessionConfig, *, timeout_s: float = 30.0) -> None:
    """Fail fast if ClickHouse isn't reachable. The stack should already be up
    and migrated (docker compose + the e2e-migrate service) before pytest runs;
    this is a connectivity gate, not a bring-up."""
    deadline = time.monotonic() + timeout_s
    last_err: Exception | None = None
    while time.monotonic() < deadline:
        try:
            ch.query(cfg, "SELECT 1")
            return
        except Exception as e:  # noqa: BLE001 — any connection error is retryable
            last_err = e
            time.sleep(0.5)
    raise RuntimeError(
        f"ClickHouse not reachable at {cfg.ch_http_url} within {timeout_s}s. "
        f"The stack must be up and migrated first (`./e2e.sh up`). Last error: {last_err}"
    )


@pytest.fixture(scope="session")
def stack_ready(session_cfg: SessionConfig) -> SessionConfig:
    """Gate: the externally-managed ClickHouse + MariaDB are reachable + migrated.

    Compose (`./e2e.sh`) brings the stack up and the `e2e-migrate` service applies
    every ClickHouse migration (core DBs, bronze placeholders, gold views) BEFORE
    pytest starts. This fixture does NOT boot or migrate anything — it only fails
    fast when the stack is missing, so a misconfigured run errors clearly instead
    of deep inside the first seed.
    """
    if _IS_PRIMARY:
        _wait_ch_ready(session_cfg)
        mariadb.wait_ready(session_cfg)
    return session_cfg


@pytest.fixture(scope="session")
def dbt_runner(stack_ready: SessionConfig):
    """Parse dbt manifest once per session; expose a runner for the world build."""
    runner = DbtRunner(stack_ready)
    runner.setup()
    yield runner
    runner.cleanup()


@pytest.fixture(scope="session")
def ch_seeder(stack_ready: SessionConfig) -> CHSeeder:
    """Session-scoped seeder used by the one-shot world build."""
    return CHSeeder(stack_ready)


@pytest.fixture(scope="session")
def enrich_runner(stack_ready: SessionConfig) -> EnrichRunner:
    """Session-scoped: discovers connector enrich steps once; builds each crate lazily."""
    return EnrichRunner(stack_ready)


@pytest.fixture(scope="session")
def all_fixtures() -> list[TestYaml]:
    """Load + resolve EVERY discovered `*.test.yaml` once, for the seed-once build.
    A malformed fixture fails the whole session here (before the stack is used)."""
    return [load_test(p) for p in discover_tests(_METRICS_ROOT)]


@pytest.fixture(scope="session")
def build_world(
    stack_ready: SessionConfig,
    dbt_runner: DbtRunner,
    enrich_runner: EnrichRunner,
    ch_seeder: CHSeeder,
    all_fixtures: list[TestYaml],
    worker_ctx: WorkerContext,
) -> SessionConfig:
    """Seed every fixture's namespaced bronze and build the whole stack ONCE.

    Assumes ClickHouse is already migrated (compose + e2e-migrate). seed_once
    resets the multi-reader tables, seeds, runs dbt + enrich, reapplies the
    gold-view migrations once (rebind to real silver), and refreshes the MVs.
    """
    if _IS_PRIMARY:
        seed_once.build_world(
            seeder=ch_seeder,
            dbt_runner=dbt_runner,
            enrich_runner=enrich_runner,
            fixtures=all_fixtures,
            worker_ctx=worker_ctx,
        )
    return stack_ready


def _collect_metrics(proc: AnalyticsApiProcess) -> None:
    """Run `lib/collect_metrics.py` (a script — NOT a test) against the
    live API, primary worker only. Snapshots the metric catalog into `.artifacts/`
    so the metric-coverage gate analyses a file with no second app boot.
    Best-effort: a failure just means the gate finds no artifact and fails loudly —
    never abort the session for it. Must run while the API is up (called from
    analytics_api teardown, before proc.stop())."""
    if not _IS_PRIMARY:
        return
    import subprocess
    import sys

    script = Path(__file__).parent / "lib" / "collect_metrics.py"
    out_dir = Path(__file__).parent / ".artifacts"
    result = subprocess.run(
        [
            sys.executable,
            str(script),
            "--url",
            proc.base_url,
            "--out-dir",
            str(out_dir),
            "--tenant",
            str(TEST_TENANT_ID),
        ],
        check=False,
    )
    if result.returncode != 0:
        LOG.warning(
            "coverage-artifact collection failed (rc=%d); gate jobs may lack inputs",
            result.returncode,
        )


@pytest.fixture(scope="session")
def analytics_api(build_world: SessionConfig):
    """Spawn the analytics-api binary baked into the runner image, AFTER the
    seed-once world is built (gold views exist, silver is populated). Its SeaORM
    migrations run on startup; we then upsert test-specific metrics from
    seed/metrics.yaml.

    If the binary is missing, this is a hard FAIL — identical locally and in CI.
    A skip here would make the whole transformation suite silently green while
    testing nothing. The binary is built FROM ITS OWN Dockerfile and baked into the
    runner image (see lib.analytics_api.locate_binary); if it isn't there the
    bronze→API tests cannot run, so the only honest result is red.
    """
    cfg = build_world
    from lib.analytics_api import ApiSpawnError  # local import to keep top clean
    try:
        binary = locate_binary(cfg)
    except ApiSpawnError as e:
        pytest.fail(f"analytics-api binary not available: {e}", pytrace=False)
    port = find_free_port()
    proc = AnalyticsApiProcess(cfg, binary, port)
    proc.start()
    seed_test_metrics(cfg)
    yield proc
    # Snapshot the metric catalog while the API is still up (a script, run via
    # subprocess — see _collect_metrics). Always
    # stop the process afterward, even if collection raised.
    try:
        _collect_metrics(proc)
    finally:
        proc.stop()


# ----------------------------------------------------------------------
# yaml-rig: per-test parametrization and execution
# ----------------------------------------------------------------------


def pytest_collection_modifyitems(config, items):
    """Convenience: order rig smoke tests (meta/ + api/) first."""
    items.sort(key=lambda i: 0 if ("meta/" in str(i.path) or "api/" in str(i.path)) else 1)


def pytest_generate_tests(metafunc):
    """Generate one `test_metric_smoke` invocation per discovered `*.test.yaml`."""
    if "test_yaml" in metafunc.fixturenames and metafunc.function.__name__ == "test_metric_smoke":
        paths = discover_tests(_METRICS_ROOT)
        metafunc.parametrize(
            "test_path",
            paths,
            ids=[p.name[: -len(".test.yaml")] for p in paths],
        )


@pytest.fixture
def test_yaml(test_path: Path) -> TestYaml:
    """Load + resolve the test file; malformed files fail here as a test failure."""
    return load_test(test_path)
