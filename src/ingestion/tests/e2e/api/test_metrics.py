"""Contract: /v1/metrics path group — definition CRUD + the two query endpoints.

  GET    /v1/metrics              200 list · 200 excludes soft-deleted
  POST   /v1/metrics              201 · 400 query_ref · 415 wrong-ct · 400 off-schema (xfail: #1670)
  GET    /v1/metrics/{id}         200 · 400 non-uuid · 404 unknown · 404 soft-deleted
  PUT    /v1/metrics/{id}         200 · 400 non-uuid · 404 unknown · 415 wrong-ct · 400 off-schema (xfail: #1670)
  DELETE /v1/metrics/{id}         204 · 400 non-uuid · 404 unknown
  POST   /v1/metrics/{id}/query   200 · 400 orderby · 404 unknown · 415 wrong-ct · 400 off-schema (xfail: #1670)
  POST   /v1/metrics/queries      200 batch · 400 malformed · 415 wrong-ct · 400 off-schema (xfail: #1670)

The scratch metric's query_ref runs the REAL engine end-to-end: parsed,
validated, wrapped (`SELECT ... FROM system.one WHERE 1=1 LIMIT n`) and
executed on ClickHouse — one deterministic row {one: 1} comes back.
"""

from __future__ import annotations

import pytest

from api.endpoint_helpers import (
    NON_UUID,
    SCRATCH_QUERY_REF,
    UNKNOWN_ID,
    create_scratch_metric,
    text_body_request,
)

pytestmark = pytest.mark.api


def test_create_metric_201(api) -> None:
    """POST /v1/metrics → 201 echoing the definition (helper asserts the body)."""
    created = create_scratch_metric(api, "e2e-scratch-create")
    api.delete(f"/v1/metrics/{created['id']}")


def test_create_metric_400_invalid_query_ref(api) -> None:
    """POST /v1/metrics → 400: query_ref is validated on write (parse_query_ref
    requires a `SELECT ... FROM` shape; a bare DROP statement has neither)."""
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
    assert updated["is_enabled"] is True, "PUT must not reset fields it was not given"


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


def test_query_metric_400_bad_orderby(api, scratch_metric: dict) -> None:
    """$orderby fields are validated against an identifier pattern (injection
    guard) — a non-identifier is a canonical 400."""
    r = api.post(
        f"/v1/metrics/{scratch_metric['id']}/query",
        json={"$orderby": "one; DROP TABLE metrics"},
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


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


def test_batch_queries_200_partial_failure(api, scratch_metric: dict) -> None:
    """A failing item does NOT fail the batch: the response stays 200 and the
    bad item carries a per-item RFC-9457 Problem (status=error envelope) while
    the good item still returns rows — the FE-consumed partial-failure
    contract."""
    r = api.post(
        "/v1/metrics/queries",
        json={
            "queries": [
                {"id": "good", "metric_id": scratch_metric["id"], "$top": 1},
                {"id": "bad", "metric_id": UNKNOWN_ID, "$top": 1},
            ]
        },
    )
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    by_id = {item["id"]: item for item in r.json()["results"]}
    assert by_id["good"]["status"] == "ok"
    assert by_id["good"]["items"] == [{"one": 1}]
    bad = by_id["bad"]
    assert bad["status"] == "error"
    assert bad["error"]["status"] == 404, bad


# ── body-parse contracts (415 wrong Content-Type, 400 off-schema) ──────────
# 415 (wrong Content-Type) is both the intended and the real status. An
# off-schema body SHOULD be a canonical 400 (as admin/catalog answer via
# CanonicalJson); the legacy `axum::Json` handlers return a non-canonical 422
# instead — #1670. The *_400_schema_mismatch cases assert the intended 400 and
# xfail(strict) until the body extractor is unified (developer error-design).


def test_create_metric_415_wrong_content_type(api) -> None:
    r = text_body_request(api, "POST", "/v1/metrics")
    assert r.status_code == 415, f"status={r.status_code} body={r.text}"


@pytest.mark.xfail(
    reason="#1670: off-schema body should be canonical 400; legacy axum::Json returns 422",
    strict=True,
)
def test_create_metric_400_schema_mismatch(api) -> None:
    """Intended: `name` is a String, a numeric value is an off-schema body → 400."""
    r = api.post("/v1/metrics", json={"name": 123, "query_ref": SCRATCH_QUERY_REF})
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_get_metric_400_non_uuid(api) -> None:
    """`{id}` binds `Path<Uuid>`; a non-UUID segment is a 400 before handler logic."""
    r = api.get(f"/v1/metrics/{NON_UUID}")
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_update_metric_400_non_uuid(api) -> None:
    r = api.put(f"/v1/metrics/{NON_UUID}", json={"name": "x"})
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_update_metric_415_wrong_content_type(api, scratch_metric: dict) -> None:
    r = text_body_request(api, "PUT", f"/v1/metrics/{scratch_metric['id']}")
    assert r.status_code == 415, f"status={r.status_code} body={r.text}"


@pytest.mark.xfail(
    reason="#1670: off-schema body should be canonical 400; legacy axum::Json returns 422",
    strict=True,
)
def test_update_metric_400_schema_mismatch(api, scratch_metric: dict) -> None:
    """Intended: `name` is `Option<String>`, a numeric value is off-schema → 400."""
    r = api.put(f"/v1/metrics/{scratch_metric['id']}", json={"name": 123})
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_delete_metric_400_non_uuid(api) -> None:
    r = api.delete(f"/v1/metrics/{NON_UUID}")
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_query_metric_415_wrong_content_type(api, scratch_metric: dict) -> None:
    r = text_body_request(api, "POST", f"/v1/metrics/{scratch_metric['id']}/query")
    assert r.status_code == 415, f"status={r.status_code} body={r.text}"


@pytest.mark.xfail(
    reason="#1670: off-schema body should be canonical 400; legacy axum::Json returns 422",
    strict=True,
)
def test_query_metric_400_schema_mismatch(api, scratch_metric: dict) -> None:
    """Intended: `$top` is a `u64`, a string value is off-schema → 400."""
    r = api.post(f"/v1/metrics/{scratch_metric['id']}/query", json={"$top": "not-a-number"})
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_batch_queries_400_malformed_json(api) -> None:
    """Malformed JSON body (correct Content-Type) is a syntax error → 400."""
    r = api.post(
        "/v1/metrics/queries",
        content=b"{not valid json",
        headers={"Content-Type": "application/json"},
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_batch_queries_415_wrong_content_type(api) -> None:
    r = text_body_request(api, "POST", "/v1/metrics/queries")
    assert r.status_code == 415, f"status={r.status_code} body={r.text}"


@pytest.mark.xfail(
    reason="#1670: off-schema body should be canonical 400; legacy axum::Json returns 422",
    strict=True,
)
def test_batch_queries_400_schema_mismatch(api) -> None:
    """Intended: `queries` is an array, a string value is off-schema → 400."""
    r = api.post("/v1/metrics/queries", json={"queries": "not-an-array"})
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
