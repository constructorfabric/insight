#!/usr/bin/env python3
"""Pinned-tenant contract guard (ADR-0003, insight#2196).

The tenant is ALWAYS the pinned per-registration value (a user attribute
stamped by each IdP mapper); an IdP's own tenancy assertions are never
consulted, and group-sourced tenants are rejected by policy. Imports the
canonical broker realm into a throwaway Keycloak and asserts:

1. clean baseline            -> the canonical realm ships zero tenant groups
2. no pinned attribute       -> token carries NO tenant_id (fail closed)
3. pinned attribute          -> token tenant_id == the pin, a single string
4. tenant-bearing group      -> must NOT influence the token (no aggregation)
                                AND the realm scan flags it as a violation

Requires docker and PyYAML. Boots quay.io/keycloak/keycloak, converts the
canonical realm YAML to a realm representation (env placeholders substituted
with synthetic values), creates it via the admin API, and evaluates example
tokens. Exit 0 = contract holds.
"""

# ruff: noqa: T201  — stdout IS this script's CI report.

import json
import re
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parents[4]
CANONICAL_REALM = REPO_ROOT / "deploy/gitops/environments/local/keycloak/realms/insight-broker.yaml"
KC_IMAGE = "quay.io/keycloak/keycloak:26.4"
KC_PORT = 18086
BASE = f"http://127.0.0.1:{KC_PORT}"
CONTAINER = "tenant-guard-kc"

PLACEHOLDER = re.compile(r"\$\(env:([A-Za-z_][A-Za-z0-9_]*)\)")
SYNTHETIC = {
    "INSIGHT_AUTHENTICATOR_CLIENT_SECRET": "guard-secret",
    "INSIGHT_AUTHENTICATOR_REDIRECT_URI": "http://localhost/auth/callback",
    "INSIGHT_TENANT_ID": "00000000-0000-0000-0000-00000000feed",
}

TENANT_GROUP = "aaaaaaaa-0000-0000-0000-000000000001"
TENANT_PIN = "cccccccc-0000-0000-0000-000000000003"


def sh(*args: str) -> None:
    subprocess.run(args, check=True, capture_output=True)


def substitute(node):
    if isinstance(node, str):
        return PLACEHOLDER.sub(lambda m: SYNTHETIC.get(m.group(1), f"missing-{m.group(1)}"), node)
    if isinstance(node, list):
        return [substitute(v) for v in node]
    if isinstance(node, dict):
        return {k: substitute(v) for k, v in node.items()}
    return node


class Admin:
    def __init__(self) -> None:
        self.token = self._call(
            "/realms/master/protocol/openid-connect/token",
            method="POST",
            raw=urllib.parse.urlencode(
                {"grant_type": "password", "client_id": "admin-cli", "username": "admin", "password": "admin"}
            ).encode(),
        )["access_token"]

    def _call(self, path: str, method: str = "GET", body=None, raw: bytes | None = None):
        headers = {"Content-Type": "application/x-www-form-urlencoded" if raw else "application/json"}
        if hasattr(self, "token"):
            headers["Authorization"] = f"Bearer {self.token}"
        req = urllib.request.Request(
            f"{BASE}{path}",
            method=method,
            headers=headers,
            data=raw if raw is not None else (json.dumps(body).encode() if body is not None else None),
        )
        # URLs are module constants targeting the throwaway 127.0.0.1 Keycloak.
        # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected
        with urllib.request.urlopen(req, timeout=30) as resp:
            payload = resp.read()
            return json.loads(payload) if payload else None

    def realm(self, path: str, method: str = "GET", body=None):
        return self._call(f"/admin/realms/insight-broker{path}", method, body)


def wait_for_keycloak(timeout_s: int = 180) -> None:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected, python.lang.security.audit.insecure-transport.urllib.insecure-urlopen.insecure-urlopen
            urllib.request.urlopen(f"{BASE}/realms/master/.well-known/openid-configuration", timeout=3)
            return
        except (urllib.error.URLError, OSError):
            time.sleep(3)
    raise TimeoutError("Keycloak did not come up")


def tenant_claim(admin: Admin, client_id: str, user_id: str):
    token = admin.realm(f"/clients/{client_id}/evaluate-scopes/generate-example-id-token?userId={user_id}&scope=openid")
    return token.get("tenant_id")


def all_groups(admin: Admin) -> list[dict]:
    groups, first, page = [], 0, 100
    while True:
        batch = admin.realm(f"/groups?first={first}&max={page}")
        groups.extend(batch)
        if len(batch) < page:
            return groups
        first += page


def tenant_bearing_groups(admin: Admin) -> list[str]:
    """Policy scan: NO group in the realm may carry a tenant_id attribute."""
    offenders = []
    for g in all_groups(admin):
        detail = admin.realm(f"/groups/{g['id']}")
        if (detail.get("attributes") or {}).get("tenant_id"):
            offenders.append(g["name"])
    return offenders


def registration_violations(realm_doc: dict) -> list[str]:
    """Static contract on a realm file: every IdP registration must pin the
    tenant (hardcoded mapper from the INSIGHT_TENANT_ID placeholder) and
    stamp idp_sub; no group may carry a tenant_id attribute."""
    problems = []
    mappers = realm_doc.get("identityProviderMappers") or []
    for idp in realm_doc.get("identityProviders") or []:
        alias = idp.get("alias", "?")
        mine = [m for m in mappers if m.get("identityProviderAlias") == alias]
        pins = [
            m
            for m in mine
            if m.get("identityProviderMapper") == "hardcoded-attribute-idp-mapper"
            and (m.get("config") or {}).get("attribute") == "tenant_id"
            and (m.get("config") or {}).get("attribute.value") == "$(env:INSIGHT_TENANT_ID)"
        ]
        if len(pins) != 1:
            problems.append(f"idp '{alias}': expected exactly one INSIGHT_TENANT_ID pin mapper, found {len(pins)}")
        if not any((m.get("config") or {}).get("user.attribute") == "idp_sub" for m in mine):
            problems.append(f"idp '{alias}': no mapper stamps the idp_sub attribute")
    for g in realm_doc.get("groups") or []:
        if (g.get("attributes") or {}).get("tenant_id"):
            problems.append(f"group '{g.get('name')}': carries a tenant_id attribute")
    return problems


def main() -> int:
    failures: list[str] = []

    def check(name: str, ok: bool, detail: str) -> None:
        print(f"{'ok  ' if ok else 'FAIL'} {name}: {detail}")
        if not ok:
            failures.append(name)

    for realm_file in sorted(REPO_ROOT.glob("deploy/gitops/environments/*/keycloak/realms/*.yaml")):
        problems = registration_violations(yaml.safe_load(realm_file.read_text()))
        check(
            f"registrations({realm_file.parent.parent.parent.name})",
            not problems,
            f"{realm_file.name}: {problems or 'contract holds'}",
        )

    realm = substitute(yaml.safe_load(CANONICAL_REALM.read_text()))

    subprocess.run(["docker", "rm", "-f", CONTAINER], capture_output=True, check=False)
    sh(
        "docker",
        "run",
        "-d",
        "--name",
        CONTAINER,
        "-p",
        f"127.0.0.1:{KC_PORT}:8080",
        "-e",
        "KC_BOOTSTRAP_ADMIN_USERNAME=admin",
        "-e",
        "KC_BOOTSTRAP_ADMIN_PASSWORD=admin",
        KC_IMAGE,
        "start-dev",
    )
    try:
        wait_for_keycloak()
        admin = Admin()
        try:
            admin._call("/admin/realms", "POST", realm)
        except urllib.error.HTTPError as e:
            check("realm-import", False, f"canonical realm rejected: HTTP {e.code} {e.read()[:200]!r}")
            return 1

        profile = admin.realm("/users/profile")
        profile["unmanagedAttributePolicy"] = "ADMIN_EDIT"
        admin.realm("/users/profile", "PUT", profile)

        check("clean-baseline", not tenant_bearing_groups(admin), "canonical realm ships zero tenant groups")

        admin.realm(
            "/users",
            "POST",
            {"username": "guard@example.com", "email": "guard@example.com", "enabled": True, "emailVerified": True},
        )
        user = admin.realm("/users?username=guard@example.com&exact=true")[0]["id"]
        client = admin.realm("/clients?clientId=insight-authenticator")[0]["id"]

        claim = tenant_claim(admin, client, user)
        check("fail-closed", claim is None, f"no pinned attribute -> claim {claim!r}")

        u = admin.realm(f"/users/{user}")
        u["attributes"] = {"tenant_id": [TENANT_PIN]}
        admin.realm(f"/users/{user}", "PUT", u)
        claim = tenant_claim(admin, client, user)
        check("pinned-tenant", claim == TENANT_PIN, f"pinned attribute -> claim {claim!r}")
        check("claim-is-scalar", isinstance(claim, str), f"claim type {type(claim).__name__}")

        admin.realm("/groups", "POST", {"name": "tenant-a", "attributes": {"tenant_id": [TENANT_GROUP]}})
        groups = {g["name"]: g["id"] for g in admin.realm("/groups")}
        admin.realm(f"/users/{user}/groups/{groups['tenant-a']}", "PUT")
        claim = tenant_claim(admin, client, user)
        check("group-inert", claim == TENANT_PIN, f"tenant group must not affect the token -> claim {claim!r}")
        check(
            "group-policy-detected",
            bool(tenant_bearing_groups(admin)),
            "a tenant-bearing group exists and the realm scan flags it",
        )
    finally:
        subprocess.run(["docker", "rm", "-f", CONTAINER], capture_output=True, check=False)

    if failures:
        print(f"\ntenant-contract guard FAILED: {failures}")
        return 1
    print("\ntenant-contract guard OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
