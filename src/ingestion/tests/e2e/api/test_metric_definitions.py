"""Contract: GET /v1/metric-definitions — the unified metric definitions listing."""

from __future__ import annotations

import pytest

pytestmark = pytest.mark.api

VALID_FORMATS = {"integer", "decimal", "currency", "percent"}
VALID_DIRECTIONS = {"higher_is_better", "lower_is_better", "neutral"}
VALID_SCHEMA_STATUSES = {"ok", "error", "unchecked"}


def test_list_metric_definitions_200(api) -> None:
    """The listing carries the seeded builtin definitions with display fields
    only. Assert 'non-empty' and field shape, not an exact count, so catalog
    growth doesn't break this contract test."""
    r = api.get("/v1/metric-definitions")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    body = r.json()
    assert body["metrics"], "seeded metric_definitions must not be empty"
    for metric in body["metrics"]:
        assert metric["metric_key"] and metric["label"]
        assert metric["format"] in VALID_FORMATS
        assert metric["direction"] in VALID_DIRECTIONS
        assert metric["schema_status"] in VALID_SCHEMA_STATUSES
        assert isinstance(metric["is_enabled"], bool)
        assert isinstance(metric["dimensions"], list)
        assert metric["last_observed_date"] is None or isinstance(
            metric["last_observed_date"], str
        )
        assert (metric["schema_status"] == "error") == (
            metric["schema_error_code"] is not None
        )


def test_list_metric_definitions_sorted_and_unique(api, tenant_override_definition) -> None:
    """Rows are sorted by metric_key ascending and each key appears once —
    a tenant override collapses onto its product-default row, and its label
    is the one returned."""
    r = api.get("/v1/metric-definitions")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    metrics = r.json()["metrics"]
    keys = [m["metric_key"] for m in metrics]
    assert keys == sorted(keys)
    assert len(keys) == len(set(keys))

    override = tenant_override_definition
    row = next(m for m in metrics if m["metric_key"] == override["metric_key"])
    assert row["label"] == override["label"]


def test_list_metric_definitions_no_computation_internals(api) -> None:
    """Computation internals (inputs, computation type, transform) stay off
    the wire — consumers get the meaning of a metric, not its implementation."""
    r = api.get("/v1/metric-definitions")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    first = r.json()["metrics"][0]
    for internal in ("computation", "inputs", "transform", "scale"):
        assert internal not in first
