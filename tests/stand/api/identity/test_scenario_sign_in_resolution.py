from __future__ import annotations

from collections.abc import Callable
from uuid import UUID

import pytest
from insight_stand import ApiClient, ApiResponse, Manifest, PersonaSession, identity_path
from pydantic import BaseModel

from ..schemas import PROBLEM_CONTENT_TYPE, IdentityValue, ProblemDocument, RoleList

BY_EXTERNAL_ID = "/internal/persons/by-external-id"
BY_EMAIL_OVERRIDE = "/internal/persons/by-email-override"
BY_ROSTER_EMAIL = "/internal/persons/by-roster-email"
PROVISION = "/internal/persons/provision"
ACTIVE_ROLES = "/internal/persons/active-roles"

SEEDED_GRANTS: dict[str, frozenset[str]] = {
    "admin_operator": frozenset({"admin"}),
    "ceo": frozenset(),
}


class ActiveRoles(BaseModel):
    person_id: UUID
    roles: list[str]


class SessionMe(BaseModel):
    email: str
    user: str
    tenant_id: str
    roles: list[str]


def _identity(response: ApiResponse) -> IdentityValue:
    assert response.status_code == 200, (
        f"{response.url}: {response.status_code} {response.text[:300]}"
    )
    value = response.parse(IdentityValue)
    assert value.insight_source_type == "person", value
    return value


@pytest.mark.requires_service_principal
@pytest.mark.requires_seed("admin_operator", "ceo")
@pytest.mark.reliability
@pytest.mark.parametrize("persona", sorted(SEEDED_GRANTS))
def test_every_route_sign_in_calls_names_the_person_and_roles_the_session_carries(
    persona: str,
    service_client: ApiClient,
    session_for: Callable[[str], PersonaSession],
    admin_operator_session: PersonaSession,
    stand_manifest: Manifest,
) -> None:
    person = stand_manifest.fixture(persona)
    idp = stand_manifest.capabilities.idp

    me_response = session_for(persona).client.get("/auth/me")
    assert me_response.status_code == 200, f"{me_response.status_code} {me_response.text[:300]}"
    me = me_response.parse(SessionMe)

    login = _identity(
        service_client.get(BY_EXTERNAL_ID, params={"source_type": idp, "external_id": person.uuid})
    )
    assert (login.value_type, login.value) == ("id", person.uuid)
    resolved = str(login.insight_source_id)
    assert resolved == me.user == person.uuid, (
        f"sign-in resolves {resolved}, the session says {me.user}, the seed minted {person.uuid}"
    )

    provisioned = _identity(
        service_client.post(
            PROVISION,
            json_body={"source_type": idp, "external_id": person.uuid, "tenant_id": me.tenant_id},
        )
    )
    assert str(provisioned.insight_source_id) == resolved, (
        "provisioning a principal the journal already binds named another person"
    )

    viewed_as = _identity(service_client.get(BY_EMAIL_OVERRIDE, params={"email": me.email}))
    assert (viewed_as.value_type, viewed_as.value) == ("email", me.email)
    assert str(viewed_as.insight_source_id) == resolved, (
        f"viewing as {me.email} lands on another person than their own sign-in"
    )

    roles_response = service_client.get(ACTIVE_ROLES, params={"person_id": resolved})
    assert roles_response.status_code == 200, (
        f"{roles_response.status_code} {roles_response.text[:300]}"
    )
    granted = roles_response.parse(ActiveRoles)
    assert str(granted.person_id) == resolved
    assert set(granted.roles) == SEEDED_GRANTS[persona], (
        f"{persona} holds {granted.roles}, the seed granted {sorted(SEEDED_GRANTS[persona])}"
    )

    if granted.roles:
        assert set(me.roles) == set(granted.roles), (
            f"the session carries {me.roles}, identity grants {granted.roles}"
        )
        return

    catalogue = admin_operator_session.client.get(identity_path("/v1/roles"))
    assert catalogue.status_code == 200, f"{catalogue.status_code} {catalogue.text[:300]}"
    role_names = {role.name for role in catalogue.parse(RoleList).items}
    assert not set(me.roles) & role_names, (
        f"{persona} holds no identity grant, yet the session carries {me.roles}"
    )


@pytest.mark.requires_service_principal
@pytest.mark.requires_seed("dev_lead")
@pytest.mark.security
@pytest.mark.parametrize("route", [BY_ROSTER_EMAIL, ACTIVE_ROLES])
def test_a_person_asking_about_themselves_is_refused_the_routes_only_sign_in_may_call(
    route: str, session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    lead = stand_manifest.fixture("dev_lead")
    params = {"email": lead.email} if route == BY_ROSTER_EMAIL else {"person_id": lead.uuid}

    response = session_for("dev_lead").client.get(identity_path(route), params=params)

    assert response.status_code == 403, (
        f"a person reached the service-only {route} ({response.status_code}): {response.text[:300]}"
    )
    assert response.content_type.startswith(PROBLEM_CONTENT_TYPE), response.content_type
    assert response.parse(ProblemDocument).status == 403
