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
  globally unique across realms, so importing the roster twice collides.

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
    args = parser.parse_args()

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

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "realm-insight.json").write_text(json.dumps(realm, indent=2, sort_keys=True) + "\n")
    (out_dir / f"realm-{args.second_realm}.json").write_text(json.dumps(second, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
