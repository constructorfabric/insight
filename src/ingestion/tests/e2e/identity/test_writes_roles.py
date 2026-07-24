"""Contract: the mutating role / person-role endpoints.

Every test restores the state it changed (grant → revoke in the same test),
so the session-scoped fixture dataset stays intact for the read tests.
Marked `mutating` so a run against a shared environment can deselect them
(`-m "identity and not mutating"`); in CI they run against the throwaway DB.
"""

from __future__ import annotations

import pytest

from identity.contract import UNPROCESSABLE_OR_CONFLICT, items_of, problem
from lib import identity_seed as seed

pytestmark = [pytest.mark.identity, pytest.mark.mutating]


# ── roles catalogue ───────────────────────────────────────────────────────


def test_role_create_delete_lifecycle(api) -> None:
    """POST /v1/roles mints a catalogue row (201); DELETE removes an unused
    one (204). The delete runs in `finally` so a failed assertion never leaks
    the scratch role into later tests."""
    r = api.post("/v1/roles", json={"name": "e2e-scratch-role"})
    assert r.status_code == 201, f"status={r.status_code} body={r.text}"
    role = r.json()
    role_id = role["role_id"]
    try:
        assert role["name"] == "e2e-scratch-role"
        listed = items_of(api.get("/v1/roles").json())
        assert any(row["role_id"] == role_id for row in listed)
    finally:
        d = api.delete(f"/v1/roles/{role_id}")
    assert d.status_code == 204, f"status={d.status_code} body={d.text}"
    listed = items_of(api.get("/v1/roles").json())
    assert not any(row["role_id"] == role_id for row in listed)


def test_role_create_duplicate_name_409(api) -> None:
    """`admin` already exists — the unique name constraint surfaces as 409."""
    r = api.post("/v1/roles", json={"name": "admin"})
    assert r.status_code == 409, f"status={r.status_code} body={r.text}"
    problem(r)


def test_role_delete_in_use_conflict(api) -> None:
    """The seeded admin role has active assignments — refusing the delete
    keeps the catalogue consistent. KNOWN DIVERGENCE: .NET 422, Rust 409."""
    r = api.delete(f"/v1/roles/{seed.ADMIN_ROLE_ID}")
    assert r.status_code in UNPROCESSABLE_OR_CONFLICT, f"status={r.status_code} body={r.text}"
    problem(r)


def test_role_create_403_non_admin(bob_api) -> None:
    r = bob_api.post("/v1/roles", json={"name": "e2e-nope"})
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"


# ── person-role assignments ───────────────────────────────────────────────


def test_person_role_grant_revoke_lifecycle(api) -> None:
    """Grant bob the admin role (201), see it active, revoke it (204), see it
    gone. The revoke runs in `finally` — a leaked second admin would silently
    defuse the last-admin lockout test below."""
    g = api.post(
        "/v1/person-roles",
        json={"person_id": str(seed.BOB), "role_id": str(seed.ADMIN_ROLE_ID)},
    )
    assert g.status_code == 201, f"status={g.status_code} body={g.text}"
    assignment_id = g.json()["person_role_id"]
    try:
        rows = items_of(api.get("/v1/person-roles").json())
        assert any(
            row.get("person_role_id") == assignment_id and row.get("valid_to") is None for row in rows
        )
    finally:
        d = api.delete(f"/v1/person-roles/{assignment_id}")
    assert d.status_code == 204, f"status={d.status_code} body={d.text}"

    rows = items_of(api.get("/v1/person-roles").json())
    active = [row for row in rows if row.get("person_role_id") == assignment_id and row.get("valid_to") is None]
    assert not active, active


def test_last_admin_revoke_locked_out(api) -> None:
    """Revoking the tenant's ONLY active admin assignment is refused — the
    lockout guard keeps the admin API reachable. KNOWN DIVERGENCE: .NET 422,
    Rust 409."""
    r = api.delete(f"/v1/person-roles/{seed.ALICE_ADMIN_ASSIGNMENT}")
    assert r.status_code in UNPROCESSABLE_OR_CONFLICT, f"status={r.status_code} body={r.text}"
    problem(r)
    # The assignment must still be active.
    rows = items_of(api.get("/v1/person-roles").json())
    assert any(
        row.get("person_role_id") == str(seed.ALICE_ADMIN_ASSIGNMENT) and row.get("valid_to") is None
        for row in rows
    )


def test_person_role_grant_403_non_admin(bob_api) -> None:
    r = bob_api.post(
        "/v1/person-roles",
        json={"person_id": str(seed.CAROL), "role_id": str(seed.ADMIN_ROLE_ID)},
    )
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"
