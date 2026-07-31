"""Session orchestrator — the central pytest conftest.

Owns the lifecycle of every session-scoped resource:

  pytest_sessionstart:
    1. docker compose up (ClickHouse + MariaDB)
    2. apply ClickHouse migrations
    3. MariaDB is migrated and seeded by `analytics migrate`, which the rig runs
       before the server (the server validates the schema, never migrates it)
    4. spawn analytics on a free loopback port

  pytest_sessionfinish:
    teardown in reverse order

All resources are exposed as session-scoped fixtures so individual tests can
consume them without touching subprocess code directly.

When pytest-xdist is active, pytest_sessionstart runs in each worker — but
docker-compose containers are shared (same names). The compose lifecycle
is therefore idempotent: subsequent workers attach to the already-running
stack. The analytics binary spawn happens in the master only (gated on
PYTEST_XDIST_WORKER) to avoid N processes on N workers.
"""

from __future__ import annotations

import logging
import os
from pathlib import Path

import pytest
from lib import compose, mariadb, session_reset
from lib.analytics import AnalyticsProcess, find_free_port, locate_binary
from lib.ch_seeder import CHSeeder
from lib.config import SessionConfig
from lib.dbt_runner import DbtRunner
from lib.enrich import EnrichRunner
from lib.fixture_loader import TestYaml, discover_tests
from lib.fixture_loader import load as load_test
from lib.identity_stub import IdentityStub
from lib.migration_applier import apply_all as apply_ch_migrations
from lib.tracked_models import TrackedModels
from lib.worker import WorkerContext

LOG = logging.getLogger("e2e.rig")


# ----------------------------------------------------------------------
# Worker-aware session lifecycle
# ----------------------------------------------------------------------

# When running under xdist, all workers share the same compose stack and the
# same analytics process. We elect the first worker as the "owner" of the
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
    """Apply ClickHouse migrations once at session start, then empty every
    fixture-data relation so a re-run over a live ClickHouse starts where a fresh
    one does."""
    cfg = compose_stack
    if _IS_PRIMARY:
        apply_ch_migrations(cfg)
        session_reset.truncate_data_tables(cfg)
    return cfg


@pytest.fixture(scope="session")
def dbt_runner(ch_migrations_applied: SessionConfig):
    """Parse dbt manifest once per session; expose a runner for per-test builds."""
    cfg = ch_migrations_applied
    runner = DbtRunner(cfg)
    runner.setup()
    yield runner
    runner.cleanup()


def _collect_metrics(cfg: SessionConfig) -> None:
    if not _IS_PRIMARY:
        return
    from lib.collect_metric_definitions import collect

    out_dir = Path(__file__).parent / ".artifacts"
    collect(cfg, out_dir)


@pytest.fixture(scope="session")
def identity_stub():
    """In-process loopback Identity stub (lib.identity_stub).

    The rig runs no Identity service, so GET /v1/persons/{person_id} would 500
    ("identity not configured"). This stub resolves one seeded person (→ 200) and
    404s the rest, so the persons endpoint exercises its real 200/404 contract
    (#1691). Started before `analytics` (which depends on it) so its URL is known
    when the binary boots — the analytics IdentityClient reads identity_url once
    at gear init.
    """
    stub = IdentityStub()
    stub.start()
    yield stub
    stub.stop()


@pytest.fixture(scope="session")
def analytics(
    ch_migrations_applied: SessionConfig, dbt_runner: DbtRunner, worker_ctx: WorkerContext, identity_stub: IdentityStub
):
    """Spawn the analytics binary baked into the runner image. Its SeaORM
    migrations run on startup.

    If the binary is missing, this is a hard FAIL — identical locally and in CI.
    A skip here would make the whole transformation suite silently green while
    testing nothing. The binary is built FROM ITS OWN Dockerfile and baked into the
    runner image (see lib.analytics.locate_binary); if it isn't there the
    bronze→API tests cannot run, so the only honest result is red.
    """
    cfg = ch_migrations_applied
    dbt_runner.run("tag:gold", worker_ctx=worker_ctx)
    from lib.analytics import ApiSpawnError  # local import to keep top clean

    try:
        binary = locate_binary(cfg)
    except ApiSpawnError as e:
        pytest.fail(f"analytics binary not available: {e}", pytrace=False)
    port = find_free_port()
    proc = AnalyticsProcess(cfg, binary, port, identity_url=identity_stub.url)
    proc.start()
    yield proc
    try:
        _collect_metrics(cfg)
    finally:
        proc.stop()


@pytest.fixture(scope="session")
def ch_seeder(ch_migrations_applied: SessionConfig) -> CHSeeder:
    """Session-scoped seeder so its ledger persists across tests in the same worker."""
    return CHSeeder(ch_migrations_applied)


@pytest.fixture
def tracked_models(dbt_runner: DbtRunner, ch_seeder: CHSeeder) -> TrackedModels:
    """Per-test dbt builds, with every relation they write registered for the
    next test to truncate."""
    return TrackedModels(dbt_runner, ch_seeder)


@pytest.fixture(scope="session")
def enrich_runner(ch_migrations_applied: SessionConfig) -> EnrichRunner:
    """Session-scoped: discovers connector enrich steps once; builds each crate lazily."""
    return EnrichRunner(ch_migrations_applied)


# ----------------------------------------------------------------------
# yaml-rig: per-test parametrization and execution
# ----------------------------------------------------------------------


_METRICS_ROOT = Path(__file__).parent / "metrics"


def pytest_collection_modifyitems(config, items):
    """Convenience: order the fast rig tests (meta/) first."""
    items.sort(key=lambda i: 0 if "meta/" in str(i.path) else 1)


def pytest_generate_tests(metafunc):
    """Generate one `test_metric_smoke` invocation per discovered `*.test.yaml`."""
    if "test_yaml" in metafunc.fixturenames and metafunc.function.__name__ == "test_metric_smoke":
        paths = discover_tests(_METRICS_ROOT)
        metafunc.parametrize("test_path", paths, ids=[p.name[: -len(".test.yaml")] for p in paths])


@pytest.fixture
def test_yaml(test_path: Path) -> TestYaml:
    """Load + resolve the test file; malformed files fail here as a test failure."""
    ty = load_test(test_path)
    if ty.skip:
        pytest.skip(ty.skip)
    return ty
