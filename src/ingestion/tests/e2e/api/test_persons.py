"""Contract: GET /v1/persons/{email} — person lookup (identity-backed).

The rig wires an in-process Identity stub (lib.identity_stub) into the analytics
config so this endpoint resolves against a real backend instead of short-circuiting
to the no-backend 500. That lets both documented outcomes be observed: 200 when the
seeded email resolves, 404 when it doesn't (#1691). The stub answers purely by
email — the analytics client sends no caller header — which is exactly the persons
handler's contract surface.
"""

from __future__ import annotations

import pytest

from lib.identity_stub import SEEDED_EMAIL, SEEDED_PERSON, UNKNOWN_EMAIL

pytestmark = pytest.mark.api


def test_person_lookup_200_found(api) -> None:
    """A seeded email resolves: the handler returns the Person body verbatim
    (analytics `get_person` → `Json(serde_json::to_value(p))`)."""
    r = api.get(f"/v1/persons/{SEEDED_EMAIL}")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    person = r.json()
    assert person["email"] == SEEDED_EMAIL
    assert person["display_name"] == SEEDED_PERSON["display_name"]
    assert person["department"] == SEEDED_PERSON["department"]
    assert person["job_title"] == SEEDED_PERSON["job_title"]
    # subordinates is a required (non-Option) field on the analytics Person;
    # a missing/renamed field would have failed deserialization → 500, not 200.
    assert person["subordinates"] == []


def test_person_lookup_404_unknown(api) -> None:
    """An email the backend doesn't know maps to a canonical 404 (the client's
    None → `PersonError::not_found`), not a 500 or an empty 200."""
    r = api.get(f"/v1/persons/{UNKNOWN_EMAIL}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"
    problem = r.json()
    assert problem.get("status") == 404
    assert problem.get("type", "").endswith("cf.core.err.not_found.v1~"), problem
    # not_found carries the looked-up email as the resource name.
    assert problem.get("context", {}).get("resource_name") == UNKNOWN_EMAIL, problem
