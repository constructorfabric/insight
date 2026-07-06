"""Contract: GET /v1/persons/{email} — person lookup (identity-backed)."""

from __future__ import annotations

import pytest

pytestmark = pytest.mark.api


def test_person_lookup_500_unconfigured(api) -> None:
    """The rig runs WITHOUT an identity service: the handler's contract for that
    topology is a canonical 500 (internally "identity resolution service not
    configured"), not a 404 — the lookup never happens. The canonical error
    layer masks server-error descriptions on the wire (the real one is
    server-log-only), so pin the internal problem-type envelope. Guards against
    a silent behavior change (e.g. an accidental 200/404 fallback)."""
    r = api.get("/v1/persons/nobody@example.com")
    assert r.status_code == 500, f"status={r.status_code} body={r.text}"
    problem = r.json()
    assert problem.get("status") == 500
    assert problem.get("type", "").endswith("cf.core.err.internal.v1~"), problem
    assert problem.get("instance") == "/v1/persons/nobody@example.com"
