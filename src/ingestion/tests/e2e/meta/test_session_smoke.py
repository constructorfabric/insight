"""Smoke tests for the test rig itself.

Verifies each session-scoped fixture in isolation, then end-to-end. Run with:

    pytest src/ingestion/tests/e2e/meta/ -m smoke

These tests are pre-MVP — they do NOT use any real fixture folder; they just
poke each layer to confirm the rig comes up cleanly.
"""

from __future__ import annotations

import pytest

from e2e_lib import clickhouse as ch
from e2e_lib import mariadb
from e2e_lib.analytics_api import AnalyticsApiProcess
from e2e_lib.config import SessionConfig


pytestmark = pytest.mark.smoke


def test_clickhouse_responds(compose_stack: SessionConfig) -> None:
    """ClickHouse answers a trivial SELECT."""
    rows = ch.query(compose_stack, "SELECT 1")
    assert rows == [(1,)]


def test_mariadb_responds(compose_stack: SessionConfig) -> None:
    """MariaDB answers SELECT 1 on the analytics database."""
    rows = mariadb.query(compose_stack, "SELECT 1")
    assert rows == [(1,)]


def test_migrations_create_insight_database(
    ch_migrations_applied: SessionConfig,
) -> None:
    """After migrations apply, the `insight` database exists with views."""
    cfg = ch_migrations_applied
    dbs = {row[0] for row in ch.query(cfg, "SHOW DATABASES")}
    assert "insight" in dbs, f"insight database missing; have: {dbs}"
    assert "identity" in dbs
    assert "person" in dbs

    views = ch.query(cfg, "SELECT name FROM system.tables WHERE database = 'insight' AND engine = 'View'")
    assert len(views) >= 20, f"expected many gold views, got {len(views)}: {views!r}"


def test_analytics_api_health(analytics_api: AnalyticsApiProcess) -> None:
    """analytics-api responds 200 on /health.

    Requires `cargo ≥ 1.92` (edition2024). Older toolchains fail at build time
    with `feature 'edition2024' is required` — run `rustup update stable`.
    """
    with analytics_api.client() as c:
        r = c.get("/health")
        assert r.status_code == 200


def test_analytics_api_lists_metrics(analytics_api: AnalyticsApiProcess) -> None:
    """SeaORM auto-migrations seed metrics on binary startup — list them."""
    with analytics_api.client() as c:
        r = c.get("/v1/metrics")
        assert r.status_code == 200, f"status={r.status_code} body={r.text}"
        payload = r.json()
        # Response shape per metric-catalog PRD: list-style or { items, page_info }
        # — accept either until we lock the contract.
        if isinstance(payload, dict) and "items" in payload:
            metrics = payload["items"]
        else:
            metrics = payload
        assert isinstance(metrics, list), f"unexpected shape: {payload!r}"
        # Prod-seed migrations add dozens of metrics; at least one is enough for smoke.
        assert len(metrics) > 0, "expected at least one auto-seeded metric"
