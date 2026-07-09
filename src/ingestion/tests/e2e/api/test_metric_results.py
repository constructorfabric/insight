"""Contract: POST /v1/metric-results — the unified-metric compute endpoint.

  POST /v1/metric-results   200 compute · 400 (empty/bad-period/unknown-key) · 415 wrong-ct

Added when the `feat/unified-metrics` merge (#1656) introduced this operation to
the committed spec. It computes results for CATALOG metrics (`metric_key` +
computation spec over the observation gold-views) — a different system from the
legacy `query_ref` metrics the `/v1/metrics` CRUD and the scratch fixtures use.

The endpoint validates its request body in `domain::metric_results::validate_request`
BEFORE touching ClickHouse, so the whole 400 family is deterministic in the rig:
an empty `metrics`, a malformed/reversed `period`, and an unknown `metric_key`
(which is a 400 via `unavailable`, NOT a 404) all reject up front. Wrong
Content-Type is a 415 at the `axum::Json` extractor.

The 200 happy-path is deferred: a deterministic compute needs a seeded catalog
metric whose observation gold-view is built in the rig, which this branch does
not yet stand up — the coverage report marks metric-results' 200 as a `✗` gap
until that fixture lands. The endpoint-coverage gate blocks on an *unexercised*
operation, not on that per-code gap, so these error-path cases are enough to
keep the new endpoint covered.
"""

from __future__ import annotations

import pytest

from api.endpoint_helpers import text_body_request

pytestmark = pytest.mark.api


def _request(*, metrics, entity_ids=("e2e-nobody@example.com",), period=("2026-01-01", "2026-01-31")):
    """A well-formed metric-results body with overridable parts."""
    return {
        "entity": {"type": "person", "ids": list(entity_ids)},
        "period": {"from": period[0], "to": period[1]},
        "metrics": metrics,
    }


def test_metric_results_400_empty_metrics(api) -> None:
    """`metrics` must not be empty — rejected by validate_request_shape before
    any ClickHouse access (deterministic, no seeded data required)."""
    r = api.post("/v1/metric-results", json=_request(metrics=[]))
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_metric_results_400_bad_period_date(api) -> None:
    """`period.from` is parsed as `%Y-%m-%d`; a non-date is a canonical 400."""
    body = _request(metrics=[{"metric_key": "ai.x", "views": [{"view": "period"}]}])
    body["period"]["from"] = "not-a-date"
    r = api.post("/v1/metric-results", json=body)
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_metric_results_400_reversed_period(api) -> None:
    """`period.from` after `period.to` is a 400 (before any bucket enumeration)."""
    body = _request(
        metrics=[{"metric_key": "ai.x", "views": [{"view": "period"}]}],
        period=("2026-02-01", "2026-01-01"),
    )
    r = api.post("/v1/metric-results", json=body)
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_metric_results_400_unknown_metric_key(api) -> None:
    """An unknown `metric_key` is resolved against the catalog and rejected as a
    400 (`unavailable`) — NOT a 404. Pins that the compute endpoint has no
    not-found path (the spec's declared 404 is `.standard_errors` boilerplate)."""
    body = _request(
        metrics=[{"metric_key": "e2e.definitely-not-a-real-metric", "views": [{"view": "period"}]}],
    )
    r = api.post("/v1/metric-results", json=body)
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_metric_results_415_wrong_content_type(api) -> None:
    """The body binds `Json<MetricResultsRequest>`; a text/plain body is a 415
    at the extractor, before the handler runs."""
    r = text_body_request(api, "POST", "/v1/metric-results")
    assert r.status_code == 415, f"status={r.status_code} body={r.text}"
