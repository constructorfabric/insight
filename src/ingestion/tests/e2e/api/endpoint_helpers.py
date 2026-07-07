"""Shared helpers for the endpoint contract tests (`api/test_*.py`)."""

from __future__ import annotations

import uuid

# A query_ref the validator accepts (SELECT ... FROM db.table, no WHERE) that
# executes deterministically on ANY ClickHouse: system.one has exactly one row.
SCRATCH_QUERY_REF = "SELECT 1 AS one FROM system.one"

# Never-created v7 UUID for the unknown-id 404 cases (no seed migration claims
# it; `test_get_metric_404_unknown` would catch one that did).
UNKNOWN_ID = "01900000-0000-7000-8000-000000000000"

# A path segment that is not a UUID, for the 400 path-parse cases: every {id}
# and {tid} route binds `Path<Uuid>`, whose deserialization failure is a 400
# (Axum `FailedToDeserializePathParams`) — before any handler logic runs.
NON_UUID = "not-a-uuid"

# A non-nil tenant that is NOT the session tenant (TEST_TENANT_ID), for the
# admin cross-tenant 403 cases: the tenant-override middleware honors a non-nil
# `X-Insight-Tenant-Id`, so re-issuing a write against another tenant's row
# hits the row-ownership check and returns 403 (`not_tenant_admin`).
OTHER_TENANT = "22222222-2222-2222-2222-222222222222"


def text_body_request(client, method: str, url: str, body: str = "{}"):
    """Issue `method url` with a `text/plain` body so the JSON body extractor
    rejects it on Content-Type — pins the 415 unsupported-media-type contract.

    415 is both intended and real everywhere. NOTE (#1670): the six
    legacy body endpoints extract with plain `axum::Json`, so this 415 carries
    Axum's non-canonical plain-text envelope (admin-crud + catalog use
    `CanonicalJson` → canonical Problem), and an off-schema body answers a
    non-canonical 422 instead of the intended canonical 400. The spec declares
    the intended contract; the 422 gap is pinned by the *_400_schema_mismatch
    xfail cases for developers to close."""
    return client.request(method, url, content=body, headers={"Content-Type": "text/plain"})


def create_scratch_metric(client, name_prefix: str) -> dict:
    """POST a scratch metric and return the created body (201 asserted).

    Callers own cleanup: soft-delete via `DELETE /v1/metrics/{id}` before the
    test ends so the scratch row never leaks into `GET /v1/metrics` listings.
    """
    r = client.post(
        "/v1/metrics",
        json={
            "name": f"{name_prefix}-{uuid.uuid4().hex[:8]}",
            "description": "e2e endpoint-contract scratch metric",
            "query_ref": SCRATCH_QUERY_REF,
        },
    )
    assert r.status_code == 201, f"create metric: status={r.status_code} body={r.text}"
    body = r.json()
    assert body["is_enabled"] is True
    assert body["query_ref"] == SCRATCH_QUERY_REF
    return body
