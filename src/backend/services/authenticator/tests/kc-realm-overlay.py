#!/usr/bin/env python3
"""Turn the generated compose realm into the e2e rig's Keycloak import set.

Input is `insight-seed-realm` output (the roster realm the compose stack imports).
The rig needs the same realm plus what only the e2e suites care about:

- a short access-token lifespan, so the authenticator's background refresher
  cycles in seconds (`e2e_refresher`) instead of minutes;
- the client's back-channel logout registration, pointing back at the
  authenticator running on the docker host (`e2e_backchannel`);
- dedicated test users, so no two suites (and no two tests inside the
  parallel `e2e_override` suite) share a principal — one test's revoke-all
  or admin disable must never reach a sibling's live session;
- a second, user-less realm (`--second-realm`) as the second issuer of the
  host-keyed map (`e2e_hostmap`). User-less because Keycloak's user ids are
  globally unique across realms, so importing the roster twice collides;
- the GitHub identity-provider registration in the documented shape
  (deploy/gitops/README.md, "Enabling GitHub sign-in") — trustEmail, the
  hardcoded tenant pin, the GitHub-id -> `idp_sub` mapper — aimed at the
  rig's GitHub stub (`--github-stub-base`/`--github-tenant-pin`, together
  or not at all; only the primary realm carries it).

Passwords and tenant ids are read off the input realm rather than restated,
so this script cannot drift from the generator.

Usage:
    python3 kc-realm-overlay.py --realm realm-insight.generated.json \
        --out-dir .artifacts/keycloak-import --second-realm insight-b \
        --backchannel-url http://host.docker.internal:8083/auth/oidc/back-channel-logout \
        --access-token-lifespan 15
"""

from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path

# One user per concern. The e2e_override suite runs its five tests in
# parallel, and view-as sessions are indexed under both the impersonator and
# the target — sharing either side lets one test's revoke-all kill another
# test's session mid-flight. Impersonators authenticate against Keycloak;
# override *targets* are resolved by the identity stub from the email alone
# and only need a realm user when the test also logs in as them.
TEST_USERS = [
    "backchannel@example.com",  # e2e_backchannel: admin logout kills its sessions
    "refresh-victim@example.com",  # e2e_refresher: disabled mid-test (invalid_grant)
    "refresh-survivor@example.com",  # e2e_refresher: must outlive the victim
    "viewer-mints@example.com",  # e2e_override: override_mints_the_session…
    "target-mints@example.com",  # e2e_override: its baseline logs in AS the target
    "viewer-relogin@example.com",  # e2e_override: override_relogin_swaps…
    "viewer-switch@example.com",  # e2e_override: override_switch_without_logout…
    "viewer-unknown@example.com",  # e2e_override: override_with_unknown_target…
    "viewer-disabled@example.com",  # e2e_override: override_is_inert_when_disabled
]


# The pair the GitHub stub validates (tests/github-stub.py).
GITHUB_CLIENT_ID = "github-e2e-client"
GITHUB_CLIENT_SECRET = "github-e2e-secret"


def _github_identity_provider(stub_base: str, tenant_pin: str) -> tuple[dict, list[dict]]:
    provider = {
        "alias": "github",
        "providerId": "github",
        "enabled": True,
        "trustEmail": True,
        "config": {
            "clientId": GITHUB_CLIENT_ID,
            "clientSecret": GITHUB_CLIENT_SECRET,
            "defaultScope": "user:email",
            # The provider's GitHub-Enterprise seam: authorize/token hang off
            # baseUrl, /user and /user/emails off apiUrl — all the stub here.
            "baseUrl": stub_base,
            "apiUrl": stub_base,
        },
    }
    mappers = [
        {
            "name": "tenant-pin",
            "identityProviderAlias": "github",
            "identityProviderMapper": "hardcoded-attribute-idp-mapper",
            "config": {"attribute": "tenant_id", "attribute.value": tenant_pin},
        },
        {
            "name": "github-id-to-idp-sub",
            "identityProviderAlias": "github",
            "identityProviderMapper": "github-user-attribute-mapper",
            "config": {"jsonField": "id", "userAttribute": "idp_sub"},
        },
    ]
    return provider, mappers


def _test_user(email: str, password: str, tenant_id: str) -> dict:
    local = email.split("@", 1)[0]
    return {
        "username": email,
        "email": email,
        "firstName": local,
        "lastName": "e2e",
        "enabled": True,
        "emailVerified": True,
        "credentials": [{"type": "password", "value": password, "temporary": False}],
        "attributes": {"org_unit": ["development"], "tenant_id": [tenant_id]},
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--realm", required=True, help="insight-seed-realm output JSON")
    parser.add_argument("--out-dir", required=True, help="Keycloak import directory")
    parser.add_argument("--second-realm", required=True, help="Name of the user-less second realm")
    parser.add_argument("--backchannel-url", required=True, help="insight-authenticator back-channel logout URL")
    parser.add_argument("--access-token-lifespan", type=int, required=True, help="Realm access-token lifespan, seconds")
    parser.add_argument("--github-stub-base", help="GitHub stub base URL as the Keycloak container reaches it")
    parser.add_argument("--github-tenant-pin", help="Hardcoded tenant_id the GitHub registration stamps")
    args = parser.parse_args()
    if bool(args.github_stub_base) != bool(args.github_tenant_pin):
        parser.error("--github-stub-base and --github-tenant-pin come together or not at all")

    realm = json.loads(Path(args.realm).read_text())

    realm["accessTokenLifespan"] = args.access_token_lifespan

    (client,) = [c for c in realm["clients"] if c["clientId"] == "insight-authenticator"]
    client.setdefault("attributes", {}).update(
        {"backchannel.logout.url": args.backchannel_url, "backchannel.logout.session.required": "true"}
    )

    seed_user = realm["users"][0]
    password = seed_user["credentials"][0]["value"]
    tenant_id = seed_user["attributes"]["tenant_id"][0]
    realm["users"].extend(_test_user(email, password, tenant_id) for email in TEST_USERS)

    second = copy.deepcopy(realm)
    second["realm"] = args.second_realm
    second["users"] = []

    # After the deepcopy: the host-keyed second realm stays broker-free.
    if args.github_stub_base:
        provider, mappers = _github_identity_provider(args.github_stub_base, args.github_tenant_pin)
        realm.setdefault("identityProviders", []).append(provider)
        realm.setdefault("identityProviderMappers", []).extend(mappers)
        # The canonical broker realm's idp_sub passthrough (the generated
        # roster realm has no brokered users, so it does not carry one):
        # KC 26 hides unmanaged user attributes from admin-API reads, so the
        # token claim is the only observable proof the GitHub mapper stamped.
        client.setdefault("protocolMappers", []).append(
            {
                "name": "idp_sub",
                "protocol": "openid-connect",
                "protocolMapper": "oidc-usermodel-attribute-mapper",
                "consentRequired": False,
                "config": {
                    "user.attribute": "idp_sub",
                    "claim.name": "idp_sub",
                    "jsonType.label": "String",
                    "id.token.claim": "true",
                    "access.token.claim": "true",
                    "userinfo.token.claim": "true",
                },
            }
        )

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "realm-insight.json").write_text(json.dumps(realm, indent=2, sort_keys=True) + "\n")
    (out_dir / f"realm-{args.second_realm}.json").write_text(json.dumps(second, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
