from __future__ import annotations

from collections.abc import Callable, Iterator
from contextlib import contextmanager

import pytest
from insight_stand import ADMIN_OPERATOR_FIXTURE, ApiClient, PersonaSession, identity_path

from .. import scratch
from ..schemas import (
    MeResponse,
    PersonListResponse,
    PersonRole,
    PersonRoleList,
    ProblemDocument,
    Role,
    RoleList,
)
from .test_admin import _admin_role_id
from .views import in_force

ME = identity_path("/v1/me")
ROLES = identity_path("/v1/roles")
PERSON_ROLES = identity_path("/v1/person-roles")
PERSONS = identity_path("/v1/persons")

GRANTEE = "support_lead"


@contextmanager
def role_grant(operator: ApiClient, person_id: str, role_id: str) -> Iterator[PersonRole]:
    created = operator.post(
        PERSON_ROLES,
        json_body={"person_id": person_id, "role_id": role_id, "reason": "stand scenario"},
    )
    assert created.status_code == 201, f"grant: {created.status_code} {created.text[:300]}"

    assignment = created.parse(PersonRole)
    assignment_id = scratch.track(
        PERSON_ROLES, "person_role_id", assignment.person_role_id, params={"person": person_id}
    )
    try:
        yield assignment
    finally:
        revoked = operator.delete(f"{PERSON_ROLES}/{assignment_id}")
        assert revoked.status_code == 204, f"revoke: {revoked.status_code} {revoked.text[:300]}"


def held_roles(client: ApiClient) -> set[tuple[str, str]]:
    response = client.get(ME)
    assert response.status_code == 200, f"me: {response.status_code} {response.text[:300]}"
    return {(str(role.role_id), role.name) for role in response.parse(MeResponse).roles}


def _catalogue(operator: ApiClient) -> set[tuple[str, str]]:
    response = operator.get(ROLES)
    assert response.status_code == 200, f"roles: {response.status_code} {response.text[:300]}"
    return {(str(role.role_id), role.name) for role in response.parse(RoleList).items}


def _person_search_status(client: ApiClient) -> int:
    response = client.get(PERSONS)
    if response.status_code == 200:
        assert response.parse(PersonListResponse).items, "the admin person search listed nobody"
    else:
        assert response.parse(ProblemDocument).status == response.status_code
    return response.status_code


@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, GRANTEE, "ceo")
@pytest.mark.security
def test_an_operator_grant_opens_the_grantees_admin_gate_and_revoking_closes_it(
    admin_operator_session: PersonaSession,
    session_for: Callable[[str], PersonaSession],
) -> None:
    operator = admin_operator_session.client
    grantee = session_for(GRANTEE)
    realm_admin = session_for("ceo")
    admin = (_admin_role_id(operator), "admin")
    name = scratch.scratch_name("role")

    created = operator.post(ROLES, json_body={"name": name})
    assert created.status_code == 201, f"create role: {created.status_code} {created.text[:300]}"
    role = created.parse(Role)
    scratch_role = (str(role.role_id), name)
    try:
        assert scratch_role in _catalogue(operator)

        duplicate = operator.post(ROLES, json_body={"name": name})
        if duplicate.status_code == 201:
            operator.delete(f"{ROLES}/{duplicate.parse(Role).role_id}")
        assert duplicate.status_code == 409, f"duplicate name: {duplicate.status_code}"
        assert duplicate.parse(ProblemDocument).status == 409

        assert held_roles(grantee.client) & {scratch_role, admin} == set()
        assert _person_search_status(grantee.client) == 403

        with role_grant(operator, grantee.person.uuid, scratch_role[0]) as assignment:
            assert in_force(assignment)
            listed = operator.get(PERSON_ROLES, params={"person": grantee.person.uuid})
            assert listed.status_code == 200, f"person-roles: {listed.status_code}"
            assert assignment.person_role_id in {
                row.person_role_id for row in listed.parse(PersonRoleList).items if in_force(row)
            }
            assert scratch_role in held_roles(grantee.client)
            assert _person_search_status(grantee.client) == 403, "a non-admin role opened the gate"

            held = operator.delete(f"{ROLES}/{role.role_id}")
            assert held.status_code == 409, f"deleting a held role: {held.status_code}"
            assert held.parse(ProblemDocument).status == 409

            with role_grant(operator, grantee.person.uuid, admin[0]):
                granted_roles = held_roles(grantee.client)
                granted_search = _person_search_status(grantee.client)
                realm_admin_search = _person_search_status(realm_admin.client)

            assert admin in granted_roles
            assert granted_search == 200, "the admin grant did not open the person search"
            assert realm_admin_search == 403, "the realm admin passed without the row"
            assert admin not in held_roles(grantee.client)
            assert _person_search_status(grantee.client) == 403, "revoking admin left it open"

        assert held_roles(grantee.client) & {scratch_role, admin} == set()

        deleted = operator.delete(f"{ROLES}/{role.role_id}")
        assert deleted.status_code == 204, f"delete role: {deleted.status_code} {deleted.text}"
        assert scratch_role not in _catalogue(operator)
    finally:
        operator.delete(f"{ROLES}/{role.role_id}")
