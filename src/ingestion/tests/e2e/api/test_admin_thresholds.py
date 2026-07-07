"""Contract: /v1/admin/metric-thresholds path group — admin threshold CRUD.

  GET    /v1/admin/metric-thresholds       200 · 400 smuggled tenant_id
  POST   /v1/admin/metric-thresholds       201 · 400 metric_id · 403 locked · 409 dup · 415 ct
  GET    /v1/admin/metric-thresholds/{id}  200 · 400 non-uuid · 404 unknown
  PUT    /v1/admin/metric-thresholds/{id}  200 · 400 non-uuid · 403 cross-tenant · 404 · 415 ct
  DELETE /v1/admin/metric-thresholds/{id}  204 · 400 non-uuid · 403 cross-tenant · 404 unknown

`metric_id` must be a `metric_catalog` row id (the `catalog_metric_id`
fixture); the admin lifecycle operates only on its own tenant-scope row — the
seeded product-default rows (tenant_id NULL) are deliberately not readable
per-id.
"""

from __future__ import annotations

import pytest

from api.conftest import purge_tenant_admin_rows
from api.endpoint_helpers import NON_UUID, OTHER_TENANT, UNKNOWN_ID, text_body_request
from lib.config import TENANT_HEADER, TEST_TENANT_ID

pytestmark = pytest.mark.api


@pytest.fixture
def locked_tenant_threshold(api, catalog_metric_id: str) -> str:
    """A LOCKED tenant-scope row on the catalog metric; returns the metric_id.

    A locked broader scope shadows narrower scopes during resolution, so a
    narrower (role) create for the same metric is refused with 403
    `threshold_locked` (DESIGN §3.6). Removed in teardown (deleting a locked row
    is allowed — it is a lock_cleared transition)."""
    purge_tenant_admin_rows(api, catalog_metric_id)
    r = api.post(
        "/v1/admin/metric-thresholds",
        json={
            "metric_id": catalog_metric_id,
            "scope": "tenant",
            "good": 0.0,
            "warn": 0.0,
            "is_locked": True,
            "lock_reason": "e2e lock-enforcer contract",
        },
    )
    assert r.status_code == 201, f"locked-row setup: status={r.status_code} body={r.text}"
    row = r.json()
    yield catalog_metric_id
    api.delete(f"/v1/admin/metric-thresholds/{row['id']}")


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


def test_create_400_unknown_metric(api) -> None:
    """Referential integrity is checked pre-write: metric_id must resolve to an
    enabled metric_catalog row."""
    r = api.post(
        "/v1/admin/metric-thresholds",
        json={"metric_id": UNKNOWN_ID, "scope": "tenant", "good": 0.0, "warn": 0.0},
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


@pytest.mark.xfail(
    reason="#1664: duplicate create answers a 500 internal (unmapped UNIQUE violation), not the declared 409",
    strict=True,
)
def test_create_409_duplicate(api, admin_threshold_row: dict) -> None:
    """A second create for the same (metric, tenant-scope) target violates
    uq_metric_threshold_scope_target — a routine client conflict that the spec
    declares as 409. Pins the contract; xfail until #1664 maps the constraint
    (today it falls through to the internal-500 schema-drift alarm)."""
    r = api.post(
        "/v1/admin/metric-thresholds",
        json={
            "metric_id": admin_threshold_row["metric_id"],
            "scope": "tenant",
            "good": 0.0,
            "warn": 0.0,
        },
    )
    assert r.status_code == 409, f"status={r.status_code} body={r.text}"
    assert r.json().get("status") == 409


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


def test_update_404_unknown(api) -> None:
    r = api.put(f"/v1/admin/metric-thresholds/{UNKNOWN_ID}", json={"good": 1.0, "warn": 1.0})
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_delete_404_unknown(api) -> None:
    r = api.delete(f"/v1/admin/metric-thresholds/{UNKNOWN_ID}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_delete_204(api, admin_threshold_row: dict) -> None:
    r = api.delete(f"/v1/admin/metric-thresholds/{admin_threshold_row['id']}")
    assert r.status_code == 204, f"status={r.status_code} body={r.text}"


# ── path-parse (400), body-parse (415), lock/cross-tenant (403) contracts ──
# Admin extracts the body with `CanonicalJson` (415 canonical, no 422) and the
# path with `Path<Uuid>` (non-UUID → 400). 403 has two sources: a broader-scope
# lock refuses a narrower create, and a write against another tenant's row fails
# the ownership check (`not_tenant_admin`).


def test_create_415_wrong_content_type(api) -> None:
    r = text_body_request(api, "POST", "/v1/admin/metric-thresholds")
    assert r.status_code == 415, f"status={r.status_code} body={r.text}"


def test_create_403_locked_broader_scope(api, locked_tenant_threshold: str) -> None:
    """A role-scope create for a metric whose tenant scope is locked is refused
    with 403 threshold_locked (the broader lock shadows the narrower write)."""
    r = api.post(
        "/v1/admin/metric-thresholds",
        json={
            "metric_id": locked_tenant_threshold,
            "scope": "role",
            "role_slug": "e2e-analyst",
            "good": 0.0,
            "warn": 0.0,
        },
    )
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"
    if r.status_code != 403:  # defensive cleanup only if a row unexpectedly landed
        api.delete(f"/v1/admin/metric-thresholds/{r.json().get('id')}")


def test_get_400_non_uuid(api) -> None:
    r = api.get(f"/v1/admin/metric-thresholds/{NON_UUID}")
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_update_400_non_uuid(api) -> None:
    r = api.put(f"/v1/admin/metric-thresholds/{NON_UUID}", json={"good": 1.0, "warn": 1.0})
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_update_415_wrong_content_type(api) -> None:
    r = text_body_request(api, "PUT", f"/v1/admin/metric-thresholds/{UNKNOWN_ID}")
    assert r.status_code == 415, f"status={r.status_code} body={r.text}"


def test_update_403_cross_tenant(api, admin_threshold_row: dict) -> None:
    """A PUT against another tenant's row fails the ownership check → 403
    (the row is loaded by id, then rejected as not_tenant_admin)."""
    r = api.put(
        f"/v1/admin/metric-thresholds/{admin_threshold_row['id']}",
        json={"good": 1.0, "warn": 1.0},
        headers={TENANT_HEADER: OTHER_TENANT},
    )
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"


def test_delete_400_non_uuid(api) -> None:
    r = api.delete(f"/v1/admin/metric-thresholds/{NON_UUID}")
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_delete_403_cross_tenant(api, admin_threshold_row: dict) -> None:
    """A DELETE against another tenant's row fails the ownership check → 403."""
    r = api.delete(
        f"/v1/admin/metric-thresholds/{admin_threshold_row['id']}",
        headers={TENANT_HEADER: OTHER_TENANT},
    )
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"
