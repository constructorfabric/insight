"""Contract: POST /v1/catalog/get_metrics — the catalog read the FE boots from."""

from __future__ import annotations

import pytest

from api.endpoint_helpers import text_body_request
from lib.config import TEST_TENANT_ID

pytestmark = pytest.mark.api


def test_get_metrics_200(api) -> None:
    """Empty context ({}) resolves through product-default/tenant: the response
    echoes the request tenant and carries the seeded catalog. Assert 'non-empty',
    not an exact count, so catalog growth doesn't break this contract test."""
    r = api.post("/v1/catalog/get_metrics", json={})
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    body = r.json()
    assert body["tenant_id"] == str(TEST_TENANT_ID)
    assert body["metrics"], "seeded metric_catalog must not be empty"
    first = body["metrics"][0]
    assert first["id"] and first["metric_key"]
    assert body["links"], "metric_query_catalog links must not be empty"


def test_get_metrics_400_unknown_field(api) -> None:
    """The request body is deny_unknown_fields — a smuggled/typo'd field is a
    canonical 400, not silently ignored (same guard the admin list pins for
    query params)."""
    r = api.post("/v1/catalog/get_metrics", json={"tenant_idd": "oops"})
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_get_metrics_415_wrong_content_type(api) -> None:
    """Catalog extracts with `CanonicalJson`; a non-JSON Content-Type is a
    canonical 415 unsupported-media-type."""
    r = text_body_request(api, "POST", "/v1/catalog/get_metrics")
    assert r.status_code == 415, f"status={r.status_code} body={r.text}"
