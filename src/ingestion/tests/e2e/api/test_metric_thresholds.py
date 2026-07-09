"""Contract: /v1/metrics/{id}/thresholds path group — legacy per-metric thresholds.

  GET    /v1/metrics/{id}/thresholds        200 · 400 non-uuid · 404 unknown metric
  POST   /v1/metrics/{id}/thresholds        201 · 400 op/404 metric · 415 ct · 400 off-schema(A)
  PUT    /v1/metrics/{id}/thresholds/{tid}  200 · 400 op/non-uuid · 404 tid · 415 · 400 off-schema(A)
  DELETE /v1/metrics/{id}/thresholds/{tid}  204 · 400 non-uuid · 404 unknown tid

KNOWN BUG #1663: every read of a non-empty thresholds table 500s (DECIMAL
value column vs f64 entity), so the success-path cases xfail — create_201
directly (strict), the rest via the scratch_threshold fixture. The error-path
cases that don't touch a stored row run for real: 404 unknown, 415 wrong
Content-Type, and the path-parse 400 (non-UUID {id}/{tid}). The off-schema body
SHOULD be a canonical 400 but returns 422 today — xfail(strict), #1670. The
400-validation cases (bad operator) DO need a stored row and so ride the #1663
xfail — the path-parse 400 covers the code in the meantime.
"""

from __future__ import annotations

import pytest

from api.endpoint_helpers import NON_UUID, UNKNOWN_ID, text_body_request

pytestmark = pytest.mark.api


@pytest.mark.xfail(
    reason="#1663: create 500s on its read-back — value is DECIMAL(20,6), entity reads f64",
    strict=True,
)
def test_create_threshold_201(api, scratch_metric: dict) -> None:
    r = api.post(
        f"/v1/metrics/{scratch_metric['id']}/thresholds",
        json={"field_name": "one", "operator": "ge", "value": 1.0, "level": "good"},
    )
    assert r.status_code == 201, f"status={r.status_code} body={r.text}"
    thr = r.json()
    assert thr["metric_id"] == scratch_metric["id"]
    assert (thr["operator"], thr["value"], thr["level"]) == ("ge", 1.0, "good")


def test_create_threshold_400_bad_operator(api, scratch_metric: dict) -> None:
    """Invalid operator rejected up-front — pins the 400 validation contract."""
    r = api.post(
        f"/v1/metrics/{scratch_metric['id']}/thresholds",
        json={"field_name": "one", "operator": "between", "value": 1.0, "level": "good"},
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_create_threshold_404_unknown_metric(api) -> None:
    """The metric is resolved (find_enabled_metric) before any validation."""
    r = api.post(
        f"/v1/metrics/{UNKNOWN_ID}/thresholds",
        json={"field_name": "one", "operator": "ge", "value": 1.0, "level": "good"},
    )
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_list_thresholds_200(api, scratch_metric: dict, scratch_threshold: dict) -> None:
    r = api.get(f"/v1/metrics/{scratch_metric['id']}/thresholds")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    assert scratch_threshold["id"] in {t["id"] for t in r.json()["items"]}


def test_list_thresholds_404_unknown_metric(api) -> None:
    r = api.get(f"/v1/metrics/{UNKNOWN_ID}/thresholds")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_update_threshold_200(api, scratch_metric: dict, scratch_threshold: dict) -> None:
    r = api.put(
        f"/v1/metrics/{scratch_metric['id']}/thresholds/{scratch_threshold['id']}",
        json={"value": 2.0, "level": "warning"},
    )
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    updated = r.json()
    assert (updated["value"], updated["level"]) == (2.0, "warning")
    # Partial update: fields absent from the request must survive.
    assert (updated["operator"], updated["field_name"]) == ("ge", "one")


def test_update_threshold_400_bad_operator(api, scratch_metric: dict, scratch_threshold: dict) -> None:
    r = api.put(
        f"/v1/metrics/{scratch_metric['id']}/thresholds/{scratch_threshold['id']}",
        json={"operator": "between"},
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_update_threshold_404_unknown_tid(api, scratch_metric: dict) -> None:
    r = api.put(
        f"/v1/metrics/{scratch_metric['id']}/thresholds/{UNKNOWN_ID}",
        json={"value": 2.0},
    )
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_delete_threshold_204(api, scratch_metric: dict, scratch_threshold: dict) -> None:
    r = api.delete(f"/v1/metrics/{scratch_metric['id']}/thresholds/{scratch_threshold['id']}")
    assert r.status_code == 204, f"status={r.status_code} body={r.text}"


def test_delete_threshold_404_unknown_tid(api, scratch_metric: dict) -> None:
    r = api.delete(f"/v1/metrics/{scratch_metric['id']}/thresholds/{UNKNOWN_ID}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


# ── path-parse (400) and body-parse (415, 400-off-schema) contracts ──────
# These reject at the extractor, BEFORE the handler reads a stored row, so they
# don't hit #1663 and need no threshold fixture. `{id}`/`{tid}` bind `Path<Uuid>`
# (non-UUID → 400); the body extractor is plain `axum::Json`: wrong Content-Type
# → 415, and an off-schema body SHOULD be a canonical 400 but returns 422 today
# (#1670) — the 400 case xfail(strict) until the extractor is unified.


def test_list_thresholds_400_non_uuid(api) -> None:
    r = api.get(f"/v1/metrics/{NON_UUID}/thresholds")
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_create_threshold_415_wrong_content_type(api) -> None:
    r = text_body_request(api, "POST", f"/v1/metrics/{UNKNOWN_ID}/thresholds")
    assert r.status_code == 415, f"status={r.status_code} body={r.text}"


@pytest.mark.xfail(
    reason="#1670: off-schema body should be canonical 400; legacy axum::Json returns 422",
    strict=True,
)
def test_create_threshold_400_schema_mismatch(api) -> None:
    """Intended: `value` is an `f64`, a string value is off-schema → 400."""
    r = api.post(
        f"/v1/metrics/{UNKNOWN_ID}/thresholds",
        json={"field_name": "one", "operator": "ge", "value": "not-a-number", "level": "good"},
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_update_threshold_415_wrong_content_type(api) -> None:
    r = text_body_request(api, "PUT", f"/v1/metrics/{UNKNOWN_ID}/thresholds/{UNKNOWN_ID}")
    assert r.status_code == 415, f"status={r.status_code} body={r.text}"


@pytest.mark.xfail(
    reason="#1670: off-schema body should be canonical 400; legacy axum::Json returns 422",
    strict=True,
)
def test_update_threshold_400_schema_mismatch(api) -> None:
    """Intended: `value` is an `Option<f64>`, a string value is off-schema → 400."""
    r = api.put(
        f"/v1/metrics/{UNKNOWN_ID}/thresholds/{UNKNOWN_ID}",
        json={"value": "not-a-number"},
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_update_threshold_400_non_uuid(api) -> None:
    """The 400-validation case (`400_bad_operator`) needs a stored row and so is
    #1663-blocked; the path-parse 400 doesn't touch a row and pins the code."""
    r = api.put(f"/v1/metrics/{NON_UUID}/thresholds/{UNKNOWN_ID}", json={"value": 2.0})
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_delete_threshold_400_non_uuid(api) -> None:
    r = api.delete(f"/v1/metrics/{NON_UUID}/thresholds/{UNKNOWN_ID}")
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
