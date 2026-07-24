"""Contract: POST /v1/profiles — resolve one identity to a person profile.

The successor read endpoint (the deprecated GET /v1/persons/{email} has no
callers and is not part of the contract). Request:
`{value_type, value, insight_source_type?, insight_source_id?}`;
value_type="email" resolves across ALL sources (source fields MUST be null),
value_type="id" resolves a source-native account id within ONE source (both
source fields REQUIRED). Visibility gates every outcome: a caller resolves
only persons in their org subtree or explicitly granted — a hidden candidate
is indistinguishable from a missing one (404).
"""

from __future__ import annotations

import pytest

from identity.contract import AMBIGUOUS_STATUSES, problem
from lib import identity_seed as seed

pytestmark = pytest.mark.identity


def _resolve_email(client, email):
    return client.post("/v1/profiles", json={"value_type": "email", "value": email})


def test_resolve_by_email_200_full_profile(api) -> None:
    """A visible subordinate resolves to the full profile: identity fields,
    tenant, supervisor projection (from org_chart + the parent's own
    observations), source-native ids, and the recursive subordinates tree."""
    r = _resolve_email(api, seed.BOB_EMAIL)
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    p = r.json()
    assert p["person_id"] == str(seed.BOB)
    assert p["email"] == seed.BOB_EMAIL
    assert p["display_name"] == "Bob Builder"
    assert p["department"] == "Engineering"
    assert p["job_title"] == "Team Lead"
    assert p["status"] == "Active"
    assert p["insight_tenant_id"] == str(seed.TEST_TENANT_ID)
    # Supervisor projection: bob's org_chart parent is alice.
    assert p.get("supervisor_email") == seed.ALICE_EMAIL
    assert p.get("supervisor_name") == "Alice Admin"
    # One current value_type='id' observation per source.
    ids = p.get("ids") or []
    assert {
        "insight_source_type": seed.SOURCE_TYPE,
        "insight_source_id": str(seed.SOURCE_ID),
        "value": "acc-bob",
    } in ids, ids
    # Recursive subordinates: carol reports to bob.
    subordinate_ids = [s["person_id"] for s in (p.get("subordinates") or [])]
    assert str(seed.CAROL) in subordinate_ids, p.get("subordinates")


def test_resolve_by_source_id_200(api) -> None:
    """value_type='id' + both source fields resolves the source-native account."""
    r = api.post(
        "/v1/profiles",
        json={
            "value_type": "id",
            "value": "acc-bob",
            "insight_source_type": seed.SOURCE_TYPE,
            "insight_source_id": str(seed.SOURCE_ID),
        },
    )
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    assert r.json()["person_id"] == str(seed.BOB)


def test_resolve_unknown_email_404(api) -> None:
    r = _resolve_email(api, seed.UNKNOWN_EMAIL)
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"
    problem(r)


def test_missing_value_type_400(api) -> None:
    r = api.post("/v1/profiles", json={"value": seed.BOB_EMAIL})
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


def test_email_with_source_fields_400(api) -> None:
    """value_type='email' forbids the source fields (they select the 'id' mode)."""
    r = api.post(
        "/v1/profiles",
        json={
            "value_type": "email",
            "value": seed.BOB_EMAIL,
            "insight_source_type": seed.SOURCE_TYPE,
            "insight_source_id": str(seed.SOURCE_ID),
        },
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


def test_ambiguous_email(api) -> None:
    """Two visible persons share the email → the data-invariant violation is
    surfaced, not silently resolved. KNOWN DIVERGENCE: .NET 422, Rust 409."""
    r = _resolve_email(api, seed.DUP_EMAIL)
    assert r.status_code in AMBIGUOUS_STATUSES, f"status={r.status_code} body={r.text}"
    problem(r)


def test_hidden_person_is_404_without_grant(api) -> None:
    """Roles ≠ visibility: alice is the tenant admin but `hidden` is outside
    her subtree and she holds no grant — indistinguishable from not-found."""
    r = _resolve_email(api, seed.HIDDEN_EMAIL)
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_hidden_person_resolves_with_explicit_grant(bob_api) -> None:
    """bob holds the seeded visibility grant on `hidden` → 200."""
    r = _resolve_email(bob_api, seed.HIDDEN_EMAIL)
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    assert r.json()["person_id"] == str(seed.HIDDEN)


def test_cross_tenant_email_404(api) -> None:
    """eve exists only in OTHER_TENANT — invisible to a TEST_TENANT caller."""
    r = _resolve_email(api, seed.EVE_EMAIL)
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_unauthenticated_401(anon_api) -> None:
    r = anon_api.post("/v1/profiles", json={"value_type": "email", "value": seed.BOB_EMAIL})
    assert r.status_code == 401, f"status={r.status_code} body={r.text}"
