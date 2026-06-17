"""Integration: analytics-api tenant-resolution failure matrix.

The API Gateway (NOT exercised by this rig — see e2e_lib/analytics_api.py header)
validates JWTs. analytics-api itself enforces TENANT ISOLATION via its tenant
middleware: every request must carry a resolvable, non-nil tenant in the
`X-Insight-Tenant-Id` header, or it is rejected before any data is read. That is
the data-correctness / security boundary that stops one tenant's request from
resolving against another tenant's data.

This matrix is the e2e mirror of the Rust unit parity tests in
`analytics-api/src/api/tenant_resolution_tests.rs`: it confirms the middleware
ACCEPTS a resolvable tenant and REJECTS every unresolvable input on EVERY read
route that traverses the middleware — the liveness probe (`/health`) and all
three data endpoints:

- `POST /v1/metrics/{id}/query`  (single-metric read)
- `POST /v1/metrics/queries`     (batch read)
- `POST /v1/catalog/get_metrics` (metric-catalog read)

Rejection cases assert the EXACT canonical envelope, not merely a 4xx/400 code:
status 400 + `application/problem+json` + `field_violations[0]` =
`{field: tenant_id, reason: TENANT_UNRESOLVED}`. Pinning the envelope (not just
the code) means an incidental 400 from body validation / Content-Type can never
masquerade as a tenant rejection, and a drift to 401/403 or a renamed reason
fails loudly. JWT auth itself lives at the gateway and is out of scope for this
analytics-api-only rig.

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
# A second, distinct, well-formed tenant — used only to send TWO different
# `X-Insight-Tenant-Id` values so the multi-valued case is genuinely "refuse to
# pick a winner", not "the same value twice".
OTHER_TENANT = "22222222-2222-2222-2222-222222222222"

# httpx accepts a header type that allows duplicate keys (list of pairs); the
# plain dict cases below cover the single-value and missing-header paths.
HeaderSpec = dict[str, str] | list[tuple[str, str]]


# Every route below traverses the tenant middleware, which resolves the tenant
# server-side and rejects an unresolvable one BEFORE the handler parses the body
# (the middleware is a layer in front of all routes — see analytics-api
# `api/mod.rs::router`). So a bad-tenant request is turned away regardless of
# body — but we still send a valid JSON body so a 400 can only come from tenant
# resolution, never from a 415 / body-validation error. `face0001`
# ("Smoke — insight.people direct") is seeded under TEST_TENANT_ID by
# seed/metrics.yaml; for the rejection cases the metric id is irrelevant.
QUERY_METRIC_ID = "00000000-0000-0000-0000-0000face0001"

# (label, method, path, json-body|None) — the full set of middleware-guarded
# read routes. The isolation boundary must hold identically on every one of
# them, not just the route that reads no data.
ROUTES: list[tuple[str, str, str, dict | None]] = [
    ("/health", "GET", "/health", None),
    (
        "/v1/metrics/{id}/query",
        "POST",
        f"/v1/metrics/{QUERY_METRIC_ID}/query",
        {"$top": 1},
    ),
    ("/v1/metrics/queries", "POST", "/v1/metrics/queries", {"queries": []}),
    ("/v1/catalog/get_metrics", "POST", "/v1/catalog/get_metrics", {}),
]


def _request(
    api: AnalyticsApiProcess, headers: HeaderSpec, method: str, path: str, body: dict | None
) -> httpx.Response:
    """Issue one request to `path` with EXACTLY `headers` (no implicit tenant)."""
    with httpx.Client(base_url=api.base_url, timeout=10.0, headers=headers) as c:
        if method == "GET":
            return c.get(path)
        return c.post(path, json=body)


def _assert_accepted_everywhere(api: AnalyticsApiProcess, headers: HeaderSpec, why: str) -> None:
    """A resolvable tenant must be admitted (200) on every middleware-guarded
    route — including the data paths where reads actually happen, not just the
    liveness probe."""
    for label, method, path, body in ROUTES:
        resp = _request(api, headers, method, path, body)
        assert resp.status_code == 200, (
            f"{why} on {label}: a resolvable tenant must be accepted (200), "
            f"got {resp.status_code} — body: {resp.text[:300]}"
        )


def _assert_tenant_unresolved_everywhere(
    api: AnalyticsApiProcess, headers: HeaderSpec, why: str
) -> None:
    """An unresolvable tenant must be rejected IDENTICALLY on every route with the
    canonical `invalid_argument` / TENANT_UNRESOLVED envelope — asserting the
    exact code AND the envelope so an incidental 400 cannot pass for a tenant
    rejection."""
    for label, method, path, body in ROUTES:
        resp = _request(api, headers, method, path, body)
        assert resp.status_code == 400, (
            f"{why} on {label}: tenant must be rejected with 400 invalid_argument "
            f"(TENANT_UNRESOLVED, tenant isolation), got {resp.status_code} — "
            f"body: {resp.text[:300]}"
        )
        # RFC 9457 canonical error — same shape the Rust parity tests pin.
        ctype = resp.headers.get("content-type", "")
        assert ctype.startswith("application/problem+json"), (
            f"{why} on {label}: rejection must be application/problem+json "
            f"(RFC 9457), got content-type {ctype!r}"
        )
        violations = resp.json().get("context", {}).get("field_violations", [])
        assert violations and violations[0].get("field") == "tenant_id", (
            f"{why} on {label}: rejection must carry a tenant_id field violation, "
            f"got {violations!r}"
        )
        assert violations[0].get("reason") == "TENANT_UNRESOLVED", (
            f"{why} on {label}: rejection reason must be TENANT_UNRESOLVED, "
            f"got {violations[0].get('reason')!r}"
        )


# --------------------------------------------------------------------------- #
# Accept: a resolvable tenant is admitted on every route.                     #
# --------------------------------------------------------------------------- #


def test_valid_tenant_is_accepted_on_every_route(analytics_api: AnalyticsApiProcess) -> None:
    """A resolvable tenant passes the middleware and reaches every route — the
    liveness probe AND all three data paths return 200 (empty items with no
    seeded bronze), proving the boundary admits a valid tenant where reads
    actually happen, not only on `/health`."""
    _assert_accepted_everywhere(
        analytics_api, {TENANT_HEADER: str(TEST_TENANT_ID)}, "valid tenant"
    )


def test_whitespace_padded_valid_tenant_is_accepted(analytics_api: AnalyticsApiProcess) -> None:
    """`read_session_tenant` trims before parsing, so a valid UUID with
    surrounding whitespace still resolves and is accepted (200). Pins the
    intentional `.trim()` branch — distinct from the whitespace-ONLY rejection
    below."""
    _assert_accepted_everywhere(
        analytics_api,
        {TENANT_HEADER: f"  {TEST_TENANT_ID}  "},
        "whitespace-padded valid tenant",
    )


# --------------------------------------------------------------------------- #
# Reject: each unresolvable input is its own test (distinct requirement,       #
# named on failure) sharing one exact-envelope assertion. Every input funnels  #
# through `auth::read_session_tenant` → None → `resolve_tenant(None)` → (no     #
# configured default in this rig) → canonical invalid_argument = 400           #
# (`auth::tenant_unresolved_response`; pinned by                               #
# `api/tenant_resolution_tests.rs`).                                           #
# --------------------------------------------------------------------------- #


def test_missing_tenant_header_is_rejected(analytics_api: AnalyticsApiProcess) -> None:
    """No `X-Insight-Tenant-Id` header → `read_session_tenant` has no first
    value → None → unresolved → 400."""
    _assert_tenant_unresolved_everywhere(analytics_api, {}, "missing header")


def test_empty_tenant_header_is_rejected(analytics_api: AnalyticsApiProcess) -> None:
    """Empty header value → `Uuid::parse_str("")` fails → None → 400."""
    _assert_tenant_unresolved_everywhere(
        analytics_api, {TENANT_HEADER: ""}, "empty header"
    )


def test_whitespace_only_tenant_header_is_rejected(analytics_api: AnalyticsApiProcess) -> None:
    """Whitespace-only header → trimmed to "" → `Uuid::parse_str` fails → None →
    400. Complements the whitespace-PADDED accept case: padding around a valid
    UUID is fine, padding around nothing is not."""
    _assert_tenant_unresolved_everywhere(
        analytics_api, {TENANT_HEADER: "   "}, "whitespace-only header"
    )


def test_nil_uuid_tenant_is_rejected(analytics_api: AnalyticsApiProcess) -> None:
    """Nil UUID is parseable but rejected by the `!is_nil()` filter → None → 400;
    a nil tenant must never pin tenant context."""
    _assert_tenant_unresolved_everywhere(
        analytics_api, {TENANT_HEADER: NIL_TENANT}, "nil uuid"
    )


def test_malformed_tenant_header_is_rejected(analytics_api: AnalyticsApiProcess) -> None:
    """Non-UUID header → `Uuid::parse_str("not-a-uuid")` fails → None → 400."""
    _assert_tenant_unresolved_everywhere(
        analytics_api, {TENANT_HEADER: "not-a-uuid"}, "malformed header"
    )


def test_multi_valued_tenant_header_is_rejected(analytics_api: AnalyticsApiProcess) -> None:
    """TWO `X-Insight-Tenant-Id` values (a header-smuggling vector) → the
    middleware refuses to pick a winner (`iter.next().is_some()`) → None → 400.
    A regression that silently bound to the first value would be a cross-tenant
    request-smuggling bug, so this must reject even though both values are
    individually well-formed."""
    _assert_tenant_unresolved_everywhere(
        analytics_api,
        [(TENANT_HEADER, str(TEST_TENANT_ID)), (TENANT_HEADER, OTHER_TENANT)],
        "multi-valued header",
    )
