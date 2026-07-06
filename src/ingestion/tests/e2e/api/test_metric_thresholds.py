"""Contract: /v1/metrics/{id}/thresholds path group — legacy per-metric thresholds.

  GET    /v1/metrics/{id}/thresholds        200 · 404 unknown metric
  POST   /v1/metrics/{id}/thresholds        201 · 400 bad operator
  PUT    /v1/metrics/{id}/thresholds/{tid}  200 · 404 unknown tid
  DELETE /v1/metrics/{id}/thresholds/{tid}  204 · 404 unknown tid
"""

from __future__ import annotations

import pytest

from api.endpoint_helpers import UNKNOWN_ID

pytestmark = pytest.mark.api


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
    assert (r.json()["value"], r.json()["level"]) == (2.0, "warning")


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
