"""Fixtures for the endpoint contract tests (`api/test_*.py`).

Every resource a case needs is a function-scoped fixture that creates the row
through the same recording client the test uses and removes it afterwards —
tests stay one-case (path, method, status code) and order-independent.
Teardown deletes are best-effort on purpose: a delete-case test already
removed its row, so a 404 there is expected, not a failure.
"""

from __future__ import annotations

import pytest

from api.endpoint_helpers import create_scratch_metric
from lib.analytics import AnalyticsProcess


@pytest.fixture
def api(analytics: AnalyticsProcess):
    """Recording httpx client (the coverage chokepoint), one per test."""
    with analytics.client() as c:
        yield c


@pytest.fixture
def scratch_metric(api) -> dict:
    """A scratch metric (`e2e-scratch-*`, deterministic system.one query_ref);
    soft-deleted in teardown so it never leaks into `GET /v1/metrics`."""
    m = create_scratch_metric(api, "e2e-scratch")
    yield m
    api.delete(f"/v1/metrics/{m['id']}")


@pytest.fixture
def scratch_threshold(api, scratch_metric: dict) -> dict:
    """A threshold (`ge 1.0 good`) on the scratch metric; removed in teardown."""
    r = api.post(
        f"/v1/metrics/{scratch_metric['id']}/thresholds",
        json={"field_name": "one", "operator": "ge", "value": 1.0, "level": "good"},
    )
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
    r = api.get(
        "/v1/admin/metric-thresholds", params={"metric_id": metric_id, "scope": "tenant"}
    )
    assert r.status_code == 200, f"admin pre-clean: status={r.status_code} body={r.text}"
    for row in r.json()["items"]:
        api.delete(f"/v1/admin/metric-thresholds/{row['id']}")


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
