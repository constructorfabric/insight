"""The four refusals identity makes to keep itself usable.

    POST   /v1/roles             409 a name already in the catalogue
    DELETE /v1/roles/{id}        409 a role somebody still holds
    DELETE /v1/person-roles/{id} 409 the tenant's last active admin
    POST   /v1/profiles          409 an email two people answer to

Analytics has no conflict path at all — every 409 it declares is boilerplate
(`coverage.py`). Identity is the opposite: these four are real, each one
protecting an invariant that a 204 would quietly break, and they are the reason
409 is not excluded on this side.

The last-admin guard is the one that matters most and the one that has to be
written most carefully. Revoking the only active admin assignment would leave
the admin API reachable by nobody — not for the rest of the run, but for the
rest of the STAND, which keeps its database until `test-stand down`. So the
test both asserts the refusal and repairs the damage if the refusal does not
happen, because a broken guard should fail one test rather than every admin
test that follows it.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, Manifest, PersonaSession, identity_path

from ..schemas import PersonRole, PersonRoleList, ProblemDocument, Role, RoleList

#: identity's own role, in `person_roles` — NOT `insight_stand.ADMIN_ROLE`,
#: which is the KEYCLOAK REALM role (`insight-admin`). They are different
#: authorities and only this one admits a caller to the admin API
#: (`test_admin.py` states that at length). Using the realm name here created a
#: role rather than colliding with one, and the sibling test below then found
#: and deleted the row this file had just made.
IDENTITY_ADMIN_ROLE = "admin"

#: The Rust service answers 409; its retired .NET predecessor answered 422 for
#: the two "still in use" cases. Both are accepted so the assertion is about
#: the REFUSAL rather than about which service is deployed — a 204 is what
#: must never happen.
REFUSED = frozenset({409, 422})


def _roles(client: ApiClient) -> RoleList:
    response = client.get(identity_path("/v1/roles"))
    assert response.status_code == 200, f"roles: {response.status_code} {response.text[:300]}"
    return response.parse(RoleList)


def _admin_role_id(client: ApiClient) -> str:
    for role in _roles(client).items:
        if role.name == IDENTITY_ADMIN_ROLE:
            return str(role.role_id)
    raise AssertionError(
        f"no {IDENTITY_ADMIN_ROLE!r} role in the catalogue: "
        f"{[r.name for r in _roles(client).items]}"
    )


def _active_admin_assignments(client: ApiClient) -> list[PersonRole]:
    role_id = _admin_role_id(client)
    response = client.get(identity_path("/v1/person-roles"))
    assert response.status_code == 200, f"person-roles: {response.status_code}"
    return [
        item
        for item in response.parse(PersonRoleList).items
        if str(item.role_id) == role_id and item.in_force
    ]


@pytest.mark.requires_seed("admin_operator")
def test_a_role_name_already_in_the_catalogue_is_409(
    admin_operator_session: PersonaSession,
) -> None:
    """The catalogue is global and addressed by name, so names must stay unique.

    `test_admin.py` finds the admin role by looking for `name == "admin"`; a
    second row with that name would make which one it finds arbitrary, and the
    grant it checks would depend on ordering.
    """
    client = admin_operator_session.client
    existing = _admin_role_id(client)  # fails loudly if the name is not taken

    response = client.post(identity_path("/v1/roles"), json_body={"name": IDENTITY_ADMIN_ROLE})
    if response.status_code == 201:
        # The constraint did not hold. Remove the duplicate before anything
        # else reads the catalogue by name, since a second row makes every
        # lookup arbitrary — including this module's own.
        created = str(response.parse(Role).role_id)
        if created != existing:
            client.delete(identity_path(f"/v1/roles/{created}"))

    assert response.status_code == 409, (
        f"a duplicate role name answered {response.status_code} rather than 409: "
        f"{response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 409


@pytest.mark.requires_seed("admin_operator")
def test_deleting_a_role_somebody_still_holds_is_refused(
    admin_operator_session: PersonaSession,
) -> None:
    """The operator's own grant is what makes this reachable.

    Deleting a role out from under an active assignment would leave a row
    pointing at nothing — the assignment survives, its role does not, and every
    gate that resolves a caller's roles has to decide what that means. Refusing
    keeps the question from arising.
    """
    client = admin_operator_session.client
    response = client.delete(identity_path(f"/v1/roles/{_admin_role_id(client)}"))

    assert response.status_code in REFUSED, (
        f"deleting a role with active assignments answered {response.status_code} — "
        f"a 204 would orphan them: {response.text[:300]}"
    )
    assert _admin_role_id(client), "the admin role was deleted despite the refusal"


@pytest.mark.requires_seed("admin_operator")
def test_revoking_the_last_active_admin_is_refused(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """The lockout guard: somebody must always be able to reach the admin API.

    Skipped rather than run when the stand happens to carry more than one
    active admin, because then the revoke is legitimate and would succeed —
    testing the guard requires being at the boundary it guards.

    Repairs itself if the guard fails. That is not tidiness: this stand keeps
    its database until it is torn down, so an unguarded revoke would take the
    admin API away from every later test AND from the next run against the same
    stand. A broken guard should cost one red test, not a dead stand.
    """
    client = admin_operator_session.client
    active = _active_admin_assignments(client)
    if len(active) != 1:
        pytest.skip(f"the stand carries {len(active)} active admin assignments, not 1")

    assignment = active[0]
    operator = stand_manifest.fixture("admin_operator")
    assert str(assignment.person_id) == operator.uuid, (
        "the only active admin is not the seeded operator — the roster changed under this test"
    )

    response = client.delete(identity_path(f"/v1/person-roles/{assignment.person_role_id}"))
    try:
        assert response.status_code in REFUSED, (
            f"revoking the tenant's only active admin answered {response.status_code} — "
            f"the admin API is now reachable by nobody: {response.text[:300]}"
        )
        assert len(_active_admin_assignments(client)) == 1, (
            "the assignment is gone despite the refusal being reported"
        )
    finally:
        if response.status_code not in REFUSED:
            # The guard did not hold. Put the grant back before anything else
            # runs, so one failure does not become every failure.
            client.post(
                identity_path("/v1/person-roles"),
                json_body={
                    "person_id": operator.uuid,
                    "role_id": str(assignment.role_id),
                    "reason": "restoring the admin grant an unguarded revoke removed",
                },
            )
