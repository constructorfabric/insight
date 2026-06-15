"""Integration: analytics-api tenant-resolution failure matrix.

The API Gateway (NOT exercised by this rig — see e2e_lib/analytics_api.py header)
validates JWTs. analytics-api itself enforces TENANT ISOLATION via its tenant
middleware: every request must carry a resolvable, non-nil tenant in the
`X-Insight-Tenant-Id` header, or it is rejected before any data is read. That is
the data-correctness / security boundary that stops one tenant's request from
resolving against another tenant's data.

This matrix confirms the middleware ACCEPTS a valid tenant and REJECTS the
missing / empty / nil / malformed cases. JWT auth itself lives at the gateway
and is out of scope for this analytics-api-only rig.

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


def test_valid_tenant_is_accepted(analytics_api: AnalyticsApiProcess) -> None:
    """Sanity: a resolvable tenant passes the middleware (200)."""
    status = _health_status(analytics_api, {TENANT_HEADER: str(TEST_TENANT_ID)})
    assert status == 200, f"valid tenant should be accepted, got {status}"


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
    status = _health_status(api, headers)
    assert status == 400, (
        f"{why}: tenant must be rejected with 400 invalid_argument "
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


def test_malformed_tenant_header_is_rejected(analytics_api: AnalyticsApiProcess) -> None:
    """Non-UUID header → `Uuid::parse_str("not-a-uuid")` fails → None → 400."""
    _assert_tenant_rejected_400(
        analytics_api, {TENANT_HEADER: "not-a-uuid"}, "malformed header"
    )
