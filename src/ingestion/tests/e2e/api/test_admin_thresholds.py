"""Contract: /v1/admin/metric-thresholds path group — admin threshold CRUD.

  GET    /v1/admin/metric-thresholds       200 · 400 smuggled tenant_id
  POST   /v1/admin/metric-thresholds       201
  GET    /v1/admin/metric-thresholds/{id}  200 · 404 unknown
  PUT    /v1/admin/metric-thresholds/{id}  200
  DELETE /v1/admin/metric-thresholds/{id}  204

`metric_id` must be a `metric_catalog` row id (the `catalog_metric_id`
fixture); the admin lifecycle operates only on its own tenant-scope row — the
seeded product-default rows (tenant_id NULL) are deliberately not readable
per-id.
"""

from __future__ import annotations

import pytest

from api.conftest import purge_tenant_admin_rows
from api.endpoint_helpers import UNKNOWN_ID
from lib.config import TEST_TENANT_ID

pytestmark = pytest.mark.api


def test_list_200(api, catalog_metric_id: str) -> None:
    r = api.get("/v1/admin/metric-thresholds", params={"metric_id": catalog_metric_id})
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    assert isinstance(r.json()["items"], list)


def test_list_400_smuggled_tenant_id(api) -> None:
    """tenant_id in the query string → canonical 400 (ListFilters is
    deny_unknown_fields; cross-tenant disclosure guard)."""
    r = api.get("/v1/admin/metric-thresholds", params={"tenant_id": str(TEST_TENANT_ID)})
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_create_201(api, catalog_metric_id: str) -> None:
    """good == warn passes the sanity-bounds gauntlet regardless of the
    metric's higher_is_better direction."""
    purge_tenant_admin_rows(api, catalog_metric_id)
    r = api.post(
        "/v1/admin/metric-thresholds",
        json={"metric_id": catalog_metric_id, "scope": "tenant", "good": 0.0, "warn": 0.0},
    )
    assert r.status_code == 201, f"status={r.status_code} body={r.text}"
    row = r.json()
    try:
        assert row["scope"] == "tenant"
        assert row["tenant_id"] == str(TEST_TENANT_ID)
        assert (row["good"], row["warn"]) == (0.0, 0.0)
    finally:
        api.delete(f"/v1/admin/metric-thresholds/{row['id']}")


def test_get_200(api, admin_threshold_row: dict) -> None:
    r = api.get(f"/v1/admin/metric-thresholds/{admin_threshold_row['id']}")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    assert r.json()["metric_id"] == admin_threshold_row["metric_id"]


def test_get_404_unknown(api) -> None:
    r = api.get(f"/v1/admin/metric-thresholds/{UNKNOWN_ID}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_update_200(api, admin_threshold_row: dict) -> None:
    r = api.put(
        f"/v1/admin/metric-thresholds/{admin_threshold_row['id']}",
        json={"good": 5.0, "warn": 5.0},
    )
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    assert (r.json()["good"], r.json()["warn"]) == (5.0, 5.0)


def test_delete_204(api, admin_threshold_row: dict) -> None:
    r = api.delete(f"/v1/admin/metric-thresholds/{admin_threshold_row['id']}")
    assert r.status_code == 204, f"status={r.status_code} body={r.text}"
