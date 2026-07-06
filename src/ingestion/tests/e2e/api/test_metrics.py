"""Contract: /v1/metrics path group — definition CRUD + the two query endpoints.

  GET    /v1/metrics              200 list · 200 excludes soft-deleted
  POST   /v1/metrics              201 create · 400 invalid query_ref
  GET    /v1/metrics/{id}         200 · 404 unknown · 404 soft-deleted
  PUT    /v1/metrics/{id}         200 · 404 unknown
  DELETE /v1/metrics/{id}         204 · 404 unknown
  POST   /v1/metrics/{id}/query   200 · 404 unknown
  POST   /v1/metrics/queries      200 batch

The scratch metric's query_ref runs the REAL engine end-to-end: parsed,
validated, wrapped (`SELECT ... FROM system.one WHERE 1=1 LIMIT n`) and
executed on ClickHouse — one deterministic row {one: 1} comes back.
"""

from __future__ import annotations

import pytest

from api.endpoint_helpers import SCRATCH_QUERY_REF, UNKNOWN_ID, create_scratch_metric

pytestmark = pytest.mark.api


def test_create_metric_201(api) -> None:
    """POST /v1/metrics → 201 echoing the definition (helper asserts the body)."""
    created = create_scratch_metric(api, "e2e-scratch-create")
    api.delete(f"/v1/metrics/{created['id']}")


def test_create_metric_400_invalid_query_ref(api) -> None:
    """POST /v1/metrics → 400: query_ref is validated on write (non-SELECT rejected)."""
    r = api.post(
        "/v1/metrics",
        json={"name": "e2e-scratch-bad", "description": "x", "query_ref": "DROP TABLE metrics"},
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_list_metrics_200(api, scratch_metric: dict) -> None:
    """GET /v1/metrics → 200 {items}: an enabled metric is listed."""
    r = api.get("/v1/metrics")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    assert scratch_metric["id"] in {m["id"] for m in r.json()["items"]}


def test_list_metrics_200_excludes_soft_deleted(api, scratch_metric: dict) -> None:
    """GET /v1/metrics → 200: a soft-deleted metric is not listed."""
    api.delete(f"/v1/metrics/{scratch_metric['id']}")
    r = api.get("/v1/metrics")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    assert scratch_metric["id"] not in {m["id"] for m in r.json()["items"]}


def test_get_metric_200(api, scratch_metric: dict) -> None:
    r = api.get(f"/v1/metrics/{scratch_metric['id']}")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    assert r.json()["name"] == scratch_metric["name"]


def test_get_metric_404_unknown(api) -> None:
    r = api.get(f"/v1/metrics/{UNKNOWN_ID}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_get_metric_404_soft_deleted(api, scratch_metric: dict) -> None:
    """Soft delete makes the id unreadable — same 404 as never-existed."""
    api.delete(f"/v1/metrics/{scratch_metric['id']}")
    r = api.get(f"/v1/metrics/{scratch_metric['id']}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_update_metric_200(api, scratch_metric: dict) -> None:
    """PUT /v1/metrics/{id} → 200; absent fields stay unchanged."""
    r = api.put(
        f"/v1/metrics/{scratch_metric['id']}",
        json={"name": scratch_metric["name"] + "-renamed", "description": "updated"},
    )
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    updated = r.json()
    assert updated["name"] == scratch_metric["name"] + "-renamed"
    assert updated["description"] == "updated"
    assert updated["query_ref"] == SCRATCH_QUERY_REF


def test_update_metric_404_unknown(api) -> None:
    r = api.put(f"/v1/metrics/{UNKNOWN_ID}", json={"name": "nope"})
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_delete_metric_204(api, scratch_metric: dict) -> None:
    r = api.delete(f"/v1/metrics/{scratch_metric['id']}")
    assert r.status_code == 204, f"status={r.status_code} body={r.text}"


def test_delete_metric_404_unknown(api) -> None:
    """Soft delete is not idempotent: an unknown id is a 404, not a no-op."""
    r = api.delete(f"/v1/metrics/{UNKNOWN_ID}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_query_metric_200(api, scratch_metric: dict) -> None:
    """POST /v1/metrics/{id}/query → 200 with the deterministic system.one row."""
    r = api.post(f"/v1/metrics/{scratch_metric['id']}/query", json={"$top": 1})
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    payload = r.json()
    assert payload["items"] == [{"one": 1}]
    assert "page_info" in payload


def test_query_metric_404_unknown(api) -> None:
    r = api.post(f"/v1/metrics/{UNKNOWN_ID}/query", json={"$top": 1})
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_batch_queries_200(api, scratch_metric: dict) -> None:
    """POST /v1/metrics/queries → 200: same engine as the single-metric query,
    per-item {status: ok} envelope (the FE's primary path — also exercised by
    every metrics/*.test.yaml, but pinned here so this module is self-contained)."""
    r = api.post(
        "/v1/metrics/queries",
        json={"queries": [{"id": "q1", "metric_id": scratch_metric["id"], "$top": 1}]},
    )
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    result = r.json()["results"][0]
    assert (result["status"], result["id"]) == ("ok", "q1")
    assert result["items"] == [{"one": 1}]
