import json
from typing import Any

import pytest

from insight_seed.keycloak_realm import build_realm


@pytest.fixture
def realm() -> dict[str, Any]:
    return build_realm("user@example.com", "example-tenant", [], "example-secret", 0)


def test_every_user_carries_the_roster_id_as_an_attribute(realm: dict[str, Any]) -> None:
    for user in realm["users"]:
        assert user["attributes"]["idp_sub"] == [user["id"]], user["username"]


@pytest.mark.parametrize("client_id", ["insight", "insight-authenticator"])
def test_both_clients_emit_the_roster_id_claim(realm: dict[str, Any], client_id: str) -> None:
    client = next(client for client in realm["clients"] if client["clientId"] == client_id)
    mapper = next(mapper for mapper in client["protocolMappers"] if mapper["name"] == "idp_sub")

    assert mapper["protocol"] == "openid-connect", client_id
    assert mapper["protocolMapper"] == "oidc-usermodel-attribute-mapper", client_id
    assert mapper["config"] == {
        "user.attribute": "idp_sub",
        "claim.name": "idp_sub",
        "jsonType.label": "String",
        "id.token.claim": "true",
        "access.token.claim": "true",
        "userinfo.token.claim": "true",
    }, client_id


def test_user_profile_preserves_attributes_for_admin_editing(realm: dict[str, Any]) -> None:
    components = realm["components"]["org.keycloak.userprofile.UserProfileProvider"]
    component = next(
        component
        for component in components
        if component["providerId"] == "declarative-user-profile"
    )
    profile = json.loads(component["config"]["kc.user.profile.config"][0])

    assert component["subComponents"] == {}
    assert {attribute["name"] for attribute in profile["attributes"]} == {
        "username",
        "email",
        "firstName",
        "lastName",
    }
    assert profile["unmanagedAttributePolicy"] == "ADMIN_EDIT"
