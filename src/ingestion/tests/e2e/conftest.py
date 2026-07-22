"""Session orchestrator — the central pytest conftest.

Owns the lifecycle of every session-scoped resource:

  pytest_sessionstart:
    1. docker compose up (ClickHouse + MariaDB)
    2. apply ClickHouse migrations
    3. MariaDB is seeded later by the analytics binary's own auto-migrations
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
from lib import clickhouse as ch
from lib import compose, mariadb
from lib.analytics import AnalyticsProcess, find_free_port, locate_binary
from lib.ch_seeder import CHSeeder
from lib.config import TEST_TENANT_ID, SessionConfig
from lib.dbt_runner import DbtRunner
from lib.enrich import EnrichRunner
from lib.fixture_loader import TestYaml, discover_tests
from lib.fixture_loader import load as load_test
from lib.identity_stub import IdentityStub
from lib.metric_seed import seed_test_metrics
from lib.migration_applier import apply_all as apply_ch_migrations
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


# Silver/staging tables that a fixture may READ via a gold view but NOT seed
# (each collab fixture seeds at most one class_collab_* table, yet
# insight.collab_bullet_rows reads all four — and each class_collab_* unions
# several per-source staging feeders). The per-test ledger only truncates what a
# fixture seeds, so on a WARM ClickHouse (re-running `./e2e.sh test` without
# `down`) the first collab fixture would inherit a prior session's rows in the
# tables it does not seed — stale rows in a dbt-rebuilt class_collab_* would skew
# its neighbours. The zoom staging models are also `incremental`/`append`, so a
# warm rebuild would ALSO accumulate duplicate unique_keys (failing their dbt
# `unique` test). Truncating these once at session start makes warm re-runs
# deterministic; CI starts fresh anyway.
_SESSION_START_TRUNCATE = [
    ("silver", "class_collab_email_activity"),
    ("silver", "class_collab_chat_activity"),
    ("silver", "class_collab_meeting_activity"),
    ("silver", "class_collab_document_activity"),
    ("staging", "m365__collab_email_activity"),
    ("staging", "m365__collab_chat_activity"),
    ("staging", "m365__collab_meeting_activity"),
    ("staging", "m365__collab_document_activity_onedrive"),
    ("staging", "m365__collab_document_activity_sharepoint"),
    # Zoom feeds class_collab_meeting_activity (cross-source meeting_hours).
    ("staging", "zoom__collab_meeting_activity"),
    ("staging", "zoom__meeting_sessions"),
    # bronze_zoom.meetings is READ (via `+zoom__collab_meeting_activity` pulling
    # zoom__meeting_sessions) by every test that seeds only participants — so it
    # is neither seed-truncated by those tests nor ledger-truncated between them
    # (derive_selectors records only models sourced from SEEDED tables). A prior
    # session's leftover rows get re-appended into zoom__meeting_sessions on
    # every such build, tripping its `unique` dbt test on the second one. Bronze
    # participants added for symmetry.
    ("bronze_zoom", "meetings"),
    ("bronze_zoom", "participants"),
    # Task-tracking: the bullet/MV chain reads class_task_* even when a fixture
    # seeds only one connector, and the enrich path writes staging.jira__task_*.
    # Reset them once at session start so warm re-runs are deterministic (CI is
    # fresh anyway); per-test TRUNCATE handles cross-test isolation.
    ("silver", "class_task_field_history"),
    ("silver", "class_task_users"),
    ("silver", "class_task_field_metadata"),
    ("silver", "class_task_worklogs"),
    ("staging", "jira__task_field_history"),
    ("staging", "jira_issue_field_snapshot"),
    ("staging", "jira_changelog_items"),
    ("staging", "jira__task_field_metadata"),
    # claude_team specs build staging.claude_team__ai_dev_usage — an incremental
    # `append` model with a dbt `unique` test on unique_key. Session-start reset
    # keeps warm re-runs (reused CH volume, no `./e2e.sh down`) from accumulating
    # duplicate keys.
    ("staging", "claude_team__ai_dev_usage"),
    # claude_team__ai_overage (cc_overage) is also incremental `append` with a
    # dbt `unique` test — reset it too for warm-rerun determinism.
    ("staging", "claude_team__ai_overage"),
    # claude_enterprise specs build staging.claude_enterprise__ai_dev_usage — an
    # incremental `append` model with a dbt `unique` test on unique_key.
    # Session-start reset keeps warm re-runs (reused CH volume, no `./e2e.sh
    # down`) from accumulating duplicate keys.
    ("staging", "claude_enterprise__ai_dev_usage"),
    # Wiki: class_wiki_pages / class_wiki_engagement are incremental
    # (delete+insert, `_version > max`) and union BOTH outline + confluence. A
    # warm re-run with the same seed _version produces no new rows, leaving the
    # prior test's data in place → cross-test contamination. Reset at session
    # start (max(_version) over the emptied table = 0, so the first test's real
    # millis _version reloads fully). CI starts fresh anyway.
    ("silver", "class_wiki_pages"),
    ("silver", "class_wiki_engagement"),
    ("silver", "class_wiki_activity"),
    # ai_smoke (cursor) builds staging.cursor__ai_dev_usage — an incremental
    # `append` model guarded by a dbt `unique` test on unique_key. Without a
    # session-start reset, a warm re-run (reused CH volume, no `./e2e.sh down`)
    # appends the same rows again → duplicate unique_keys → the unique test fails.
    ("staging", "cursor__ai_dev_usage"),
    # chatgpt_team specs build staging.chatgpt_team__ai_dev_usage (codex) and
    # staging.chatgpt_team__ai_assistant_usage (chat) — both incremental `append`
    # models with a dbt `unique` test on unique_key. Session-start reset keeps
    # warm re-runs (reused CH volume, no `./e2e.sh down`) from accumulating
    # duplicate keys.
    ("staging", "chatgpt_team__ai_dev_usage"),
    ("staging", "chatgpt_team__ai_assistant_usage"),
]


@pytest.fixture(scope="session")
def ch_migrations_applied(compose_stack: SessionConfig) -> SessionConfig:
    """Apply ClickHouse migrations once at session start, then reset the
    multi-reader silver/staging tables so warm re-runs are deterministic."""
    cfg = compose_stack
    if _IS_PRIMARY:
        apply_ch_migrations(cfg)
        for schema, table in _SESSION_START_TRUNCATE:
            ch.execute(cfg, f"TRUNCATE TABLE IF EXISTS `{schema}`.`{table}`")
    return cfg


@pytest.fixture(scope="session")
def dbt_runner(ch_migrations_applied: SessionConfig):
    """Parse dbt manifest once per session; expose a runner for per-test builds."""
    cfg = ch_migrations_applied
    runner = DbtRunner(cfg)
    runner.setup()
    yield runner
    runner.cleanup()


def _collect_metrics(proc: AnalyticsProcess) -> None:
    """Run `lib/collect_metrics.py` (a script — NOT a test) against the
    live API, primary worker only. Snapshots the metric catalog into `.artifacts/`
    so the metric-coverage gate analyses a file with no second app boot.
    Best-effort: a failure just means the gate finds no artifact and fails loudly —
    never abort the session for it. Must run while the API is up (called from
    analytics teardown, before proc.stop())."""
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
            "--bearer",
            proc.bearer(str(TEST_TENANT_ID)),
        ],
        check=False,
    )
    if result.returncode != 0:
        LOG.warning("coverage-artifact collection failed (rc=%d); gate jobs may lack inputs", result.returncode)


@pytest.fixture(scope="session")
def identity_stub():
    """In-process loopback Identity stub (lib.identity_stub).

    The rig runs no Identity service, so GET /v1/persons/{email} would 500
    ("identity not configured"). This stub resolves one seeded email (→ 200) and
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
def analytics(ch_migrations_applied: SessionConfig, identity_stub: IdentityStub):
    """Spawn the analytics binary baked into the runner image. Its SeaORM
    migrations run on startup; we then upsert test-specific metrics from
    seed/metrics.yaml.

    If the binary is missing, this is a hard FAIL — identical locally and in CI.
    A skip here would make the whole transformation suite silently green while
    testing nothing. The binary is built FROM ITS OWN Dockerfile and baked into the
    runner image (see lib.analytics.locate_binary); if it isn't there the
    bronze→API tests cannot run, so the only honest result is red.
    """
    cfg = ch_migrations_applied
    from lib.analytics import ApiSpawnError  # local import to keep top clean

    try:
        binary = locate_binary(cfg)
    except ApiSpawnError as e:
        pytest.fail(f"analytics binary not available: {e}", pytrace=False)
    port = find_free_port()
    proc = AnalyticsProcess(cfg, binary, port, identity_url=identity_stub.url)
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


@pytest.fixture(scope="session")
def ch_seeder(ch_migrations_applied: SessionConfig) -> CHSeeder:
    """Session-scoped seeder so its ledger persists across tests in the same worker."""
    return CHSeeder(ch_migrations_applied)


@pytest.fixture(scope="session")
def enrich_runner(ch_migrations_applied: SessionConfig) -> EnrichRunner:
    """Session-scoped: discovers connector enrich steps once; builds each crate lazily."""
    return EnrichRunner(ch_migrations_applied)


# ----------------------------------------------------------------------
# yaml-rig: per-test parametrization and execution
# ----------------------------------------------------------------------


_METRICS_ROOT = Path(__file__).parent / "metrics"


def pytest_collection_modifyitems(config, items):
    """Convenience: order the fast rig/contract tests (meta/ + api/) first."""
    items.sort(key=lambda i: 0 if ("meta/" in str(i.path) or "api/" in str(i.path)) else 1)


def pytest_sessionfinish(session, exitstatus):
    """Dump the API-endpoint ledger recorded by the httpx response hook in
    `AnalyticsProcess.client()` (`lib.api_coverage.record_response`).

    The endpoint-coverage gate (`lib/api_coverage.py`, run by `./e2e.sh gates`
    and the api-endpoint-coverage-gate CI job via the coverage-inputs-api
    artifact) diffs this against the committed OpenAPI spec. Primary worker only;
    best-effort — a failed artifact write is logged, never fails the run (a
    missing ledger then fails the downstream gate loudly instead)."""
    if not _IS_PRIMARY:
        return
    from lib import api_coverage

    out = Path(__file__).parent / ".artifacts" / "observed_endpoints.json"
    try:
        api_coverage.dump_observed(out)
        LOG.info("wrote API-endpoint ledger (%d ops): %s", len(api_coverage._OBSERVED), out)
    except OSError as e:
        LOG.warning("could not write API-endpoint ledger %s: %s", out, e)


def pytest_generate_tests(metafunc):
    """Generate one `test_metric_smoke` invocation per discovered `*.test.yaml`."""
    if "test_yaml" in metafunc.fixturenames and metafunc.function.__name__ == "test_metric_smoke":
        paths = discover_tests(_METRICS_ROOT)
        metafunc.parametrize("test_path", paths, ids=[p.name[: -len(".test.yaml")] for p in paths])


@pytest.fixture
def test_yaml(test_path: Path) -> TestYaml:
    """Load + resolve the test file; malformed files fail here as a test failure."""
    return load_test(test_path)
