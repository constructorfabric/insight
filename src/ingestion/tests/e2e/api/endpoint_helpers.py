"""Shared helpers for the endpoint contract tests (`api/test_*.py`)."""

from __future__ import annotations

import uuid

# A query_ref the validator accepts (SELECT ... FROM db.table, no WHERE) that
# executes deterministically on ANY ClickHouse: system.one has exactly one row.
SCRATCH_QUERY_REF = "SELECT 1 AS one FROM system.one"

# Never-created v7 UUID for the unknown-id 404 cases (no seed migration claims
# it; `test_get_metric_404_unknown` would catch one that did).
UNKNOWN_ID = "01900000-0000-7000-8000-000000000000"


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
