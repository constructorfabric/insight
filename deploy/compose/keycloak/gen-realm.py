#!/usr/bin/env python3
"""Generate the `insight` Keycloak realm from the seeded 25-person org.

Reads the same roster builder the DB seeder uses
(`deploy/seed/profiles.py::build_roster`) so every user in the realm
matches a row in `identity.persons`, then emits an importable Keycloak
realm JSON: 25 users, the `insight` + `insight-authenticator` clients,
their 5 shared protocol mappers, the 4 team groups + `executive`, and
the 3 realm roles.

Usage:
    python3 gen-realm.py --out deploy/compose/keycloak/realm-insight.generated.json
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

# `deploy/seed` is a sibling package (not installed), so it has to be put on
# sys.path explicitly to import `profiles` from this script's location.
_SEED_DIR = Path(__file__).resolve().parents[2] / "seed"
sys.path.insert(0, str(_SEED_DIR))

from profiles import Person, build_roster, get_dev_user_email  # noqa: E402

REALM_NAME = "insight"
DEV_PASSWORD = "insight-dev"

TEAMS = ["development", "sales", "hr", "support", "executive"]

REALM_ROLES = ["insight-admin", "insight-lead", "insight-member"]

# Person.role -> realm roles the user is granted.
_ROLE_TO_REALM_ROLES: dict[str, list[str]] = {
    "ceo": ["insight-admin", "insight-lead"],
    "lead": ["insight-lead"],
    "ic": ["insight-member"],
}


def _org_unit(person: Person) -> str:
    """CEO has no team (Person.team is None) → the literal 'executive'."""
    return person.team if person.team is not None else "executive"


def _protocol_mappers(tenant_id: str) -> list[dict]:
    """The 5 shared mappers, identical on both clients (Input table)."""
    common = {"id.token.claim": "true", "access.token.claim": "true", "userinfo.token.claim": "true"}
    return [
        {
            # The authenticator reads this single-string claim (idp.tenant_claim,
            # default `tenant_id`) — one and only one tenant per token — and it
            # becomes the gateway JWT's `tenant_id`.
            "name": "tenant_id",
            "protocol": "openid-connect",
            "protocolMapper": "oidc-hardcoded-claim-mapper",
            "consentRequired": False,
            "config": {**common, "claim.name": "tenant_id", "claim.value": tenant_id, "jsonType.label": "String"},
        },
        {
            "name": "org_unit",
            "protocol": "openid-connect",
            "protocolMapper": "oidc-usermodel-attribute-mapper",
            "consentRequired": False,
            "config": {**common, "user.attribute": "org_unit", "claim.name": "org_unit", "jsonType.label": "String"},
        },
        {
            "name": "groups",
            "protocol": "openid-connect",
            "protocolMapper": "oidc-group-membership-mapper",
            "consentRequired": False,
            "config": {**common, "claim.name": "groups", "full.path": "false"},
        },
        {
            "name": "roles",
            "protocol": "openid-connect",
            "protocolMapper": "oidc-usermodel-realm-role-mapper",
            "consentRequired": False,
            "config": {**common, "claim.name": "roles", "jsonType.label": "String", "multivalued": "true"},
        },
        {
            "name": "aud-insight",
            "protocol": "openid-connect",
            "protocolMapper": "oidc-audience-mapper",
            "consentRequired": False,
            "config": {"id.token.claim": "false", "access.token.claim": "true", "included.client.audience": "insight"},
        },
    ]


def _client_insight(tenant_id: str) -> dict:
    return {
        "clientId": "insight",
        "publicClient": True,
        "protocol": "openid-connect",
        "standardFlowEnabled": True,
        "directAccessGrantsEnabled": True,
        "serviceAccountsEnabled": False,
        # localhost:3000 = the compose frontend's callback (SPA does the OIDC
        # code+PKCE flow directly). localhost:8080 kept for the orbstack/SPA
        # host that shares this realm.
        "redirectUris": ["http://localhost:3000/callback", "http://localhost:8080/callback"],
        "webOrigins": ["+"],
        "attributes": {"pkce.code.challenge.method": "S256"},
        "protocolMappers": _protocol_mappers(tenant_id),
    }


def _client_insight_authenticator(tenant_id: str, redirect_uris: list[str], secret: str) -> dict:
    return {
        "clientId": "insight-authenticator",
        "publicClient": False,
        "protocol": "openid-connect",
        "secret": secret,
        "standardFlowEnabled": True,
        "directAccessGrantsEnabled": False,
        "serviceAccountsEnabled": False,
        # The nginx+auth authenticator does the server-side code exchange and
        # sets the __Host-sid cookie at the callback, so the redirect must be the
        # browser-facing SPA/gateway origin (NOT the authenticator's own :8083).
        # Compose: the Vite SPA (:3000) and the gateway edge (:8080), both of
        # which proxy /auth to the authenticator. k8s passes its ingress host.
        "redirectUris": redirect_uris,
        "webOrigins": [],
        "protocolMappers": _protocol_mappers(tenant_id),
    }


def _user(person: Person) -> dict:
    org_unit = _org_unit(person)
    return {
        "id": person.uuid,
        "username": person.email,
        "email": person.email,
        "firstName": person.first_name,
        "lastName": person.last_name,
        "enabled": True,
        "emailVerified": True,
        "credentials": [{"type": "password", "value": DEV_PASSWORD, "temporary": False}],
        "attributes": {"org_unit": [org_unit]},
        "groups": [f"/{org_unit}"],
        "realmRoles": _ROLE_TO_REALM_ROLES[person.role],
    }


DEFAULT_AUTHENTICATOR_REDIRECTS = [
    "http://localhost:3000/auth/callback",  # compose Vite SPA origin
    "http://localhost:8080/auth/callback",  # compose gateway edge (curl/e2e)
]
DEFAULT_AUTHENTICATOR_SECRET = "insight-authenticator-dev-secret"


def build_realm(
    dev_user_email: str, tenant_id: str, authenticator_redirects: list[str], authenticator_secret: str
) -> dict:
    roster = build_roster(dev_user_email)
    return {
        "realm": REALM_NAME,
        "enabled": True,
        "sslRequired": "none",
        "roles": {"realm": [{"name": name} for name in REALM_ROLES]},
        "groups": [{"name": team, "path": f"/{team}"} for team in TEAMS],
        "users": [_user(person) for person in roster],
        "clients": [
            _client_insight(tenant_id),
            _client_insight_authenticator(tenant_id, authenticator_redirects, authenticator_secret),
        ],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, help="Output path for the realm JSON")
    parser.add_argument(
        "--dev-email",
        default=None,
        help=(
            "Roster-anchor email for the dev-lead persona. Decouples the realm's "
            "roster anchor from VITE_DEV_USER_EMAIL (which the frontend/gateway also "
            "read as the impersonation trigger). Falls back to VITE_DEV_USER_EMAIL "
            "when omitted."
        ),
    )
    parser.add_argument(
        "--authenticator-redirect",
        action="append",
        default=None,
        dest="authenticator_redirects",
        help=(
            "Registered redirect URI for the insight-authenticator (BFF) client. "
            "Repeatable. The nginx+auth authenticator sets the session cookie at "
            "the callback, so this must be the browser-facing SPA/gateway origin "
            "(e.g. http://localhost:3000/auth/callback in compose, or the ingress "
            "host + /auth/callback in k8s). Defaults to the compose origins."
        ),
    )
    parser.add_argument(
        "--authenticator-secret",
        default=DEFAULT_AUTHENTICATOR_SECRET,
        help="Confidential client secret for insight-authenticator (dev default).",
    )
    args = parser.parse_args()

    # Explicit --dev-email wins; otherwise fall back to VITE_DEV_USER_EMAIL via
    # get_dev_user_email() (which fail-fasts if that is also unset).
    dev_user_email = args.dev_email if args.dev_email else get_dev_user_email()
    # Same fallback value as deploy/seed/identity.py's run() — that script's
    # own TENANT_DEFAULT_ID lookup carries the identical default, so this
    # mirrors (rather than introduces) that convention.
    tenant_id = os.environ.get(  # RULE-DEFAULTS-OK: mirrors deploy/seed/identity.py's TENANT_DEFAULT_ID default, so the realm's tenant_id claim converges with an un-configured seed run
        "TENANT_DEFAULT_ID", "00000000-df51-5b42-9538-d2b56b7ee953"
    )

    redirects = args.authenticator_redirects or DEFAULT_AUTHENTICATOR_REDIRECTS
    realm = build_realm(dev_user_email, tenant_id, redirects, args.authenticator_secret)

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(realm, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
