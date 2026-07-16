"""Fixtures for the endpoint contract tests (`api/test_*.py`).

Every resource a case needs is a function-scoped fixture that creates the row
through the same recording client the test uses and removes it afterwards —
tests stay one-case (path, method, status code) and order-independent.
Teardown deletes are best-effort on purpose: a delete-case test already
removed its row, so a 404 there is expected, not a failure.
"""

from __future__ import annotations

import uuid

import pytest
from lib import mariadb
from lib.analytics import AnalyticsProcess
from lib.config import SessionConfig

from api.endpoint_helpers import create_scratch_metric


@pytest.fixture
def api(analytics: AnalyticsProcess):
    """Recording httpx client (the coverage chokepoint), one per test."""
    with analytics.client() as c:
        yield c


@pytest.fixture
def other_tenant_headers(analytics) -> dict:
    """`Authorization` for a DIFFERENT tenant — overrides the client's default
    bearer to exercise cross-tenant 403s (the signed `tenant_id` is the tenant
    authority now that `X-Insight-Tenant-Id` is gone)."""
    from api.endpoint_helpers import OTHER_TENANT

    return {"Authorization": f"Bearer {analytics.bearer(OTHER_TENANT)}"}


@pytest.fixture
def scratch_metric(api) -> dict:
    """A scratch metric (`e2e-scratch-*`, deterministic system.one query_ref);
    soft-deleted in teardown so it never leaks into `GET /v1/metrics`."""
    m = create_scratch_metric(api, "e2e-scratch")
    yield m
    api.delete(f"/v1/metrics/{m['id']}")


@pytest.fixture
def scratch_threshold(api, scratch_metric: dict) -> dict:
    """A threshold (`ge 1.0 good`) on the scratch metric; removed in teardown.

    #1663 xfail: the create's read-back 500s (DECIMAL value vs f64 entity), so
    this fixture xfails its dependents until the fix lands — at which point the
    xfail stops firing and the newly-observed success code trips BLOCKED's
    now-observed hygiene in lib/api_coverage.py, forcing this scaffolding out.
    """
    r = api.post(
        f"/v1/metrics/{scratch_metric['id']}/thresholds",
        json={"field_name": "one", "operator": "ge", "value": 1.0, "level": "good"},
    )
    if r.status_code == 500:
        pytest.xfail("#1663: threshold create 500s on read-back (DECIMAL value vs f64 entity)")
    assert r.status_code == 201, f"threshold setup: status={r.status_code} body={r.text}"
    thr = r.json()
    yield thr
    api.delete(f"/v1/metrics/{scratch_metric['id']}/thresholds/{thr['id']}")


@pytest.fixture
def catalog_metric_id(api) -> str:
    """A real `metric_catalog` row id — admin thresholds validate against it."""
    r = api.post("/v1/catalog/get_metrics", json={})
    assert r.status_code == 200, f"catalog setup: status={r.status_code} body={r.text}"
    return r.json()["metrics"][0]["id"]


def purge_tenant_admin_rows(api, metric_id: str) -> None:
    """Drop tenant-scope admin-threshold leftovers for this metric.

    Local-rerun hygiene: a persistent MariaDB volume keeps prior rows, and the
    (metric, tenant, scope) composite is UNIQUE — a fresh create would 409.
    """
    r = api.get("/v1/admin/metric-thresholds", params={"metric_id": metric_id, "scope": "tenant"})
    assert r.status_code == 200, f"admin pre-clean: status={r.status_code} body={r.text}"
    for row in r.json()["items"]:
        api.delete(f"/v1/admin/metric-thresholds/{row['id']}")


@pytest.fixture
def seeded_columns(session_cfg: SessionConfig) -> dict:
    """Two `table_columns` rows in two distinct tables, inserted directly into
    MariaDB (there is no write endpoint for this catalog — it is operator/
    migration-seeded in production, and no seed migration exists), removed in
    teardown. Gives /v1/columns/{table} a non-empty universe so the per-table
    filter is asserted against data, not vacuously against an empty set.
    tenant NULL = platform-visible (the handler shows NULL-tenant rows to any
    tenant)."""
    tag = uuid.uuid4().hex[:8]
    rows = {
        "table_a": f"e2e_cols_{tag}_a",
        "table_b": f"e2e_cols_{tag}_b",
        "ids": [uuid.uuid4().hex.upper(), uuid.uuid4().hex.upper()],
    }
    mariadb.query(
        session_cfg,
        "INSERT INTO table_columns (id, insight_tenant_id, clickhouse_table, field_name) VALUES "
        f"(UNHEX('{rows['ids'][0]}'), NULL, '{rows['table_a']}', 'metric_value'), "
        f"(UNHEX('{rows['ids'][1]}'), NULL, '{rows['table_b']}', 'other_value')",
    )
    yield rows
    mariadb.query(
        session_cfg, f"DELETE FROM table_columns WHERE id IN (UNHEX('{rows['ids'][0]}'), UNHEX('{rows['ids'][1]}'))"
    )


@pytest.fixture
def admin_threshold_row(api, catalog_metric_id: str) -> dict:
    """An own tenant-scope admin threshold row; removed in teardown.

    good == warn passes the sanity-bounds gauntlet regardless of the metric's
    higher_is_better direction.
    """
    purge_tenant_admin_rows(api, catalog_metric_id)
    r = api.post(
        "/v1/admin/metric-thresholds",
        json={"metric_id": catalog_metric_id, "scope": "tenant", "good": 0.0, "warn": 0.0},
    )
    assert r.status_code == 201, f"admin row setup: status={r.status_code} body={r.text}"
    row = r.json()
    yield row
    api.delete(f"/v1/admin/metric-thresholds/{row['id']}")
