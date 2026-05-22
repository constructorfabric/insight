"""Session orchestrator — the central pytest conftest.

Owns the lifecycle of every session-scoped resource:

  pytest_sessionstart:
    1. docker compose up (ClickHouse + MariaDB)
    2. apply ClickHouse migrations
    3. MariaDB is seeded later by the analytics-api binary's own auto-migrations
    4. spawn analytics-api on a free loopback port

  pytest_sessionfinish:
    teardown in reverse order

All resources are exposed as session-scoped fixtures so individual tests can
consume them without touching subprocess code directly.

When pytest-xdist is active, pytest_sessionstart runs in each worker — but
docker-compose containers are shared (same names). The compose lifecycle
is therefore idempotent: subsequent workers attach to the already-running
stack. The analytics-api binary spawn happens in the master only (gated on
PYTEST_XDIST_WORKER) to avoid N processes on N workers.
"""

from __future__ import annotations

import logging
import os
import pytest

from pathlib import Path

from e2e_lib import clickhouse as ch
from e2e_lib import compose, mariadb
from e2e_lib.analytics_api import AnalyticsApiProcess, build, find_free_port
from e2e_lib.ch_seeder import CHSeeder
from e2e_lib.config import SessionConfig
from e2e_lib.csv_asserter import assert_matches, update_snapshot
from e2e_lib.dbt_runner import DbtRunner
from e2e_lib.fixture_loader import Fixture, discover_all, load as load_fixture
from e2e_lib.metric_seed import seed_test_metrics
from e2e_lib.migration_applier import apply_all as apply_ch_migrations
from e2e_lib.worker import WorkerContext

LOG = logging.getLogger("e2e.rig")


# ----------------------------------------------------------------------
# Worker-aware session lifecycle
# ----------------------------------------------------------------------

# When running under xdist, all workers share the same compose stack and the
# same analytics-api process. We elect the first worker as the "owner" of the
# shared resources; the others wait until the owner reports ready.
#
# For the scaffolding MVP we keep it simple: do NOT support xdist yet (the
# scaffold smoke test is serial). Parallel safety lands with the dbt-runner
# feature where per-worker schema suffix becomes meaningful.

_IS_XDIST = bool(os.environ.get("PYTEST_XDIST_WORKER"))
_IS_PRIMARY = not _IS_XDIST or os.environ.get("PYTEST_XDIST_WORKER") == "gw0"


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


@pytest.fixture(scope="session")
def compose_stack(session_cfg: SessionConfig):
    """docker compose up at session start, down at session end.

    In `host` mode (default): pytest brings compose up and tears it down.
    In `docker` mode: compose was started by the parent (./e2e.sh) — we just
    verify CH+MariaDB respond and skip the teardown.

    Yields the SessionConfig for downstream fixtures' convenience.
    """
    in_docker = session_cfg.run_mode == "docker"
    if _IS_PRIMARY and not in_docker:
        compose.up(session_cfg)
    if _IS_PRIMARY:
        mariadb.wait_ready(session_cfg)
    yield session_cfg
    if _IS_PRIMARY and not in_docker:
        if os.environ.get("E2E_KEEP_CONTAINERS") != "1":
            compose.down(session_cfg, remove_volumes=True)
        else:
            LOG.info("E2E_KEEP_CONTAINERS=1 — leaving containers up")


@pytest.fixture(scope="session")
def ch_migrations_applied(compose_stack: SessionConfig) -> SessionConfig:
    """Apply ClickHouse migrations once at session start."""
    cfg = compose_stack
    if _IS_PRIMARY:
        apply_ch_migrations(cfg)
    return cfg


@pytest.fixture(scope="session")
def dbt_runner(ch_migrations_applied: SessionConfig):
    """Parse dbt manifest once per session; expose a runner for per-test builds."""
    cfg = ch_migrations_applied
    runner = DbtRunner(cfg)
    runner.setup()
    yield runner
    runner.cleanup()


@pytest.fixture(scope="session")
def analytics_api(ch_migrations_applied: SessionConfig):
    """Build + spawn the analytics-api binary. Its SeaORM migrations run on startup;
    we then upsert any test-specific metrics from seed/metrics.yaml.

    If `cargo build` fails (e.g. cargo < 1.92 lacks edition2024), every test
    that requires this fixture is skipped — the rest of the framework still
    runs against the data plane.
    """
    cfg = ch_migrations_applied
    from e2e_lib.analytics_api import ApiSpawnError  # local import to keep top clean
    try:
        binary = build(cfg)
    except ApiSpawnError as e:
        pytest.skip(f"analytics-api binary not buildable: {e}")
    port = find_free_port()
    proc = AnalyticsApiProcess(cfg, binary, port)
    proc.start()
    seed_test_metrics(cfg)
    yield proc
    proc.stop()


@pytest.fixture(scope="session")
def ch_seeder(ch_migrations_applied: SessionConfig) -> CHSeeder:
    """Session-scoped seeder so its ledger persists across tests in the same worker."""
    return CHSeeder(ch_migrations_applied)


# ----------------------------------------------------------------------
# csv-rig: per-fixture parametrization and execution
# ----------------------------------------------------------------------


_FIXTURES_ROOT = Path(__file__).parent / "fixtures"


def pytest_collection_modifyitems(config, items):
    """Convenience: order smoke tests under meta/ first."""
    items.sort(key=lambda i: 0 if "meta/" in str(i.path) else 1)


def _all_fixture_paths() -> list[Path]:
    """Discover candidate fixture folders. Eagerly load to catch malformed
    fixtures at pytest-collect time (per cpt-bronze-to-api-e2e-dod-csv-rig-folder-discovery)."""
    paths = discover_all(_FIXTURES_ROOT)
    # Validate each so a misshapen fixture fails collection of just itself,
    # not the whole session. Errors are re-raised by the test function below
    # via pytest.fail() — we cannot raise here (collection-time exceptions
    # abort the whole module).
    return paths


def pytest_generate_tests(metafunc):
    """Generate one `test_fixture` invocation per fixture folder."""
    if "fixture" in metafunc.fixturenames and metafunc.function.__name__ == "test_fixture":
        paths = _all_fixture_paths()
        metafunc.parametrize(
            "fixture_path",
            paths,
            ids=[p.name for p in paths],
        )


@pytest.fixture
def fixture(fixture_path: Path) -> Fixture:
    """Load the fixture; failures surface as test failures (not collection errors)."""
    return load_fixture(fixture_path)


@pytest.fixture
def update_snapshots(pytestconfig) -> bool:
    """`--update-snapshots` CLI flag — feature-snapshot-update plumbing."""
    return bool(pytestconfig.getoption("--update-snapshots", default=False))


def pytest_addoption(parser):
    parser.addoption(
        "--update-snapshots",
        action="store_true",
        default=False,
        help="Write actual response to expected/response.csv instead of asserting. "
             "Refuses to run under CI=true.",
    )
