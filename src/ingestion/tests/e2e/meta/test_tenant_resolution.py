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


@pytest.mark.parametrize(
    "case,headers",
    [
        ("missing", {}),
        ("empty", {TENANT_HEADER: ""}),
        ("nil-uuid", {TENANT_HEADER: NIL_TENANT}),
        ("malformed", {TENANT_HEADER: "not-a-uuid"}),
    ],
)
def test_unresolvable_tenant_is_rejected(
    analytics_api: AnalyticsApiProcess, case: str, headers: dict[str, str]
) -> None:
    """Tenant isolation: a missing / empty / nil / malformed tenant must be
    REJECTED with a 4xx — never silently served, which would let a caller read
    data without a resolvable tenant scope."""
    status = _health_status(analytics_api, headers)
    assert 400 <= status < 500, (
        f"{case} tenant must be rejected with 4xx (tenant isolation), got {status}"
    )
