"""Contract: the admin-gated read endpoints — roles / person-roles / visibility.

All three lists are gated by the same admin check (caller = JWT `sub`, must
hold an active `admin` assignment in the JWT tenant): non-admin → 403,
unauthenticated → 401.
"""

from __future__ import annotations

import pytest

from identity.contract import items_of, list_response, problem
from lib import identity_seed as seed

pytestmark = pytest.mark.identity


# ── GET /v1/roles ─────────────────────────────────────────────────────────


def test_roles_list_200_contains_admin(api) -> None:
    r = api.get("/v1/roles")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    # Strict wire envelope: {"items": [...], "next_cursor": null|str}.
    roles, cursor = list_response(r.json())
    assert cursor is None, cursor
    by_id = {row["role_id"]: row for row in roles}
    assert str(seed.ADMIN_ROLE_ID) in by_id, roles
    assert by_id[str(seed.ADMIN_ROLE_ID)]["name"] == "admin"


def test_roles_list_403_non_admin(bob_api) -> None:
    r = bob_api.get("/v1/roles")
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"
    problem(r)


def test_roles_list_401_unauthenticated(anon_api) -> None:
    assert anon_api.get("/v1/roles").status_code == 401


# ── GET /v1/person-roles ──────────────────────────────────────────────────


def test_person_roles_list_200_contains_seeded_admin(api) -> None:
    r = api.get("/v1/person-roles")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    rows = items_of(r.json())
    match = [
        row
        for row in rows
        if row.get("person_id") == str(seed.ALICE) and row.get("role_id") == str(seed.ADMIN_ROLE_ID)
    ]
    assert match, rows
    assert match[0].get("valid_to") is None, match[0]


def test_person_roles_list_403_non_admin(bob_api) -> None:
    r = bob_api.get("/v1/person-roles")
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"


def test_person_roles_list_401_unauthenticated(anon_api) -> None:
    assert anon_api.get("/v1/person-roles").status_code == 401


# ── GET /v1/visibility ────────────────────────────────────────────────────


def test_visibility_list_200_contains_seeded_grant(api) -> None:
    r = api.get("/v1/visibility")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    rows = items_of(r.json())
    match = [
        row
        for row in rows
        if row.get("viewer_person_id") == str(seed.BOB) and row.get("viewed_person_id") == str(seed.HIDDEN)
    ]
    assert match, rows


def test_visibility_list_403_non_admin(bob_api) -> None:
    r = bob_api.get("/v1/visibility")
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"


def test_visibility_list_401_unauthenticated(anon_api) -> None:
    assert anon_api.get("/v1/visibility").status_code == 401
