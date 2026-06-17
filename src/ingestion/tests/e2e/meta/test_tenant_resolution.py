"""Integration: analytics-api tenant-resolution failure matrix.

The API Gateway (NOT exercised by this rig — see e2e_lib/analytics_api.py header)
validates JWTs. analytics-api itself enforces TENANT ISOLATION via its tenant
middleware: every request must carry a resolvable, non-nil tenant in the
`X-Insight-Tenant-Id` header, or it is rejected before any data is read. That is
the data-correctness / security boundary that stops one tenant's request from
resolving against another tenant's data.

This matrix confirms the middleware ACCEPTS a valid tenant and REJECTS the
missing / empty / nil / malformed cases on every read route that traverses the
tenant middleware — the liveness probe (`/health`) and all three data endpoints:
`POST /v1/metrics/{id}/query` (single), `POST /v1/metrics/queries` (batch), and
`POST /v1/catalog/get_metrics` (catalog). The isolation boundary must hold on
every route, not just the one that reads no data. JWT auth itself lives at the
gateway and is out of scope for this analytics-api-only rig.

Run:
    pytest src/ingestion/tests/e2e/meta/test_tenant_resolution.py -m smoke
"""

from __future__ import annotations

import httpx
import pytest

from e2e_lib.analytics_api import AnalyticsApiProcess
from e2e_lib.config import TENANT_HEADER, TEST_TENANT_ID

pytestmark = pytest.mark.smoke

NIL_TENANT = "00000000-0000-0000-0000-000000000000"


def _health_status(api: AnalyticsApiProcess, headers: dict[str, str]) -> int:
    """GET /health with exactly the given headers (the simplest endpoint that
    still traverses the tenant middleware)."""
    with httpx.Client(base_url=api.base_url, timeout=5.0, headers=headers) as c:
        return c.get("/health").status_code


# The three data endpoints below all traverse the tenant middleware, which
# resolves the tenant server-side and rejects an unresolvable one BEFORE the
# handler parses the body (see analytics-api `catalog.rs` header). So a
# bad-tenant request is turned away regardless of body — but we still send a
# valid Content-Type + body so a 400 can only come from tenant resolution, never
# from a 415 or body-validation error. `face0001` ("Smoke — insight.people
# direct") is seeded under TEST_TENANT_ID by seed/metrics.yaml; for the rejection
# cases the metric id is irrelevant.
QUERY_METRIC_ID = "00000000-0000-0000-0000-0000face0001"
_QUERY_BODY = {"$top": 1}
_BATCH_BODY = {"queries": []}
_CATALOG_BODY: dict = {}


def _post_status(
    api: AnalyticsApiProcess, headers: dict[str, str], path: str, body: dict
) -> int:
    """POST `path` with a JSON body and exactly the given headers."""
    with httpx.Client(base_url=api.base_url, timeout=10.0, headers=headers) as c:
        return c.post(path, json=body).status_code


def _query_status(api: AnalyticsApiProcess, headers: dict[str, str]) -> int:
    """POST /v1/metrics/{id}/query — single-metric read path."""
    return _post_status(
        api, headers, f"/v1/metrics/{QUERY_METRIC_ID}/query", _QUERY_BODY
    )


def _batch_query_status(api: AnalyticsApiProcess, headers: dict[str, str]) -> int:
    """POST /v1/metrics/queries — batch read path."""
    return _post_status(api, headers, "/v1/metrics/queries", _BATCH_BODY)


def _catalog_status(api: AnalyticsApiProcess, headers: dict[str, str]) -> int:
    """POST /v1/catalog/get_metrics — metric-catalog read path."""
    return _post_status(api, headers, "/v1/catalog/get_metrics", _CATALOG_BODY)


def test_valid_tenant_is_accepted(analytics_api: AnalyticsApiProcess) -> None:
    """Sanity: a resolvable tenant passes the middleware on /health (200)."""
    status = _health_status(analytics_api, {TENANT_HEADER: str(TEST_TENANT_ID)})
    assert status == 200, f"valid tenant should be accepted, got {status}"


def test_valid_tenant_is_accepted_on_query_endpoint(
    analytics_api: AnalyticsApiProcess,
) -> None:
    """A resolvable tenant reaches the DATA path, not just liveness: POST to the
    metrics-query endpoint returns 200 (empty items with no seeded bronze),
    proving the boundary admits a valid tenant where reads actually happen."""
    status = _query_status(analytics_api, {TENANT_HEADER: str(TEST_TENANT_ID)})
    assert status == 200, f"valid tenant should reach the query endpoint, got {status}"


def test_valid_tenant_is_accepted_on_batch_query_endpoint(
    analytics_api: AnalyticsApiProcess,
) -> None:
    """A resolvable tenant reaches the batch read path: an (empty) batch query
    returns 200, proving the boundary admits a valid tenant on /v1/metrics/queries."""
    status = _batch_query_status(analytics_api, {TENANT_HEADER: str(TEST_TENANT_ID)})
    assert status == 200, f"valid tenant should reach the batch endpoint, got {status}"


def test_valid_tenant_is_accepted_on_catalog_endpoint(
    analytics_api: AnalyticsApiProcess,
) -> None:
    """A resolvable tenant reaches the catalog read path: get_metrics returns 200,
    proving the boundary admits a valid tenant on /v1/catalog/get_metrics."""
    status = _catalog_status(analytics_api, {TENANT_HEADER: str(TEST_TENANT_ID)})
    assert status == 200, (
        f"valid tenant should reach the catalog endpoint, got {status}"
    )


# Each rejection case below is its own test rather than one parametrized matrix:
# they are distinct requirements (different inputs, potentially different codes
# if the spec ever diverges), so a per-case test pins each independently and a
# failure names the exact case. They share one assertion of the EXACT code,
# because today every unresolvable input funnels through
# `auth::read_session_tenant` → `None` → `resolve_tenant(None)` → (no configured
# default in this rig) → the canonical `invalid_argument` envelope = 400
# (`auth::tenant_unresolved_response`; pinned by `api/tenant_resolution_tests.rs`).
# Asserting `== 400` (not a 4xx range) catches a drift to 401/403 that would mean
# the isolation boundary or the envelope moved.


def _assert_tenant_rejected_400(
    api: AnalyticsApiProcess, headers: dict[str, str], why: str
) -> None:
    # The boundary must hold on EVERY route, so assert both the liveness probe
    # and the real data path (the metrics-query endpoint) reject identically.
    for probe, where in (
        (_health_status, "/health"),
        (_query_status, "/v1/metrics/{id}/query"),
        (_batch_query_status, "/v1/metrics/queries"),
        (_catalog_status, "/v1/catalog/get_metrics"),
    ):
        status = probe(api, headers)
        assert status == 400, (
            f"{why} on {where}: tenant must be rejected with 400 invalid_argument "
            f"(TENANT_UNRESOLVED, tenant isolation), got {status}"
        )


def test_missing_tenant_header_is_rejected(analytics_api: AnalyticsApiProcess) -> None:
    """No `X-Insight-Tenant-Id` header → `read_session_tenant` has no first
    value → None → unresolved → 400."""
    _assert_tenant_rejected_400(analytics_api, {}, "missing header")


def test_empty_tenant_header_is_rejected(analytics_api: AnalyticsApiProcess) -> None:
    """Empty header value → `Uuid::parse_str("")` fails → None → 400."""
    _assert_tenant_rejected_400(analytics_api, {TENANT_HEADER: ""}, "empty header")


def test_nil_uuid_tenant_is_rejected(analytics_api: AnalyticsApiProcess) -> None:
    """Nil UUID is parseable but rejected by the `!is_nil()` filter → None → 400;
    a nil tenant must never pin tenant context."""
    _assert_tenant_rejected_400(analytics_api, {TENANT_HEADER: NIL_TENANT}, "nil uuid")


def test_malformed_tenant_header_is_rejected(
    analytics_api: AnalyticsApiProcess,
) -> None:
    """Non-UUID header → `Uuid::parse_str("not-a-uuid")` fails → None → 400."""
    _assert_tenant_rejected_400(
        analytics_api, {TENANT_HEADER: "not-a-uuid"}, "malformed header"
    )
