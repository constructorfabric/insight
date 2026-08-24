"""Seed manifest — the machine-readable description of a seeded stand.

Written to `manifest.json` in the working directory, which for the compose
seed container is the bind-mounted seeder directory the stand suite reads
(`/app/manifest.json`). `SEED_MANIFEST_PATH` names it explicitly — a cluster
Job does, because its filesystem is discarded and the log is the record.

The builder is a PURE FUNCTION of (roster, supplied env, committed
constants). It queries neither MariaDB nor ClickHouse. That is what lets the
same document be rendered without a running stack — which the PROFILE.md
staleness check depends on — and what makes two runs at the same anchor
byte-identical.

`build_manifest` never reads `os.environ` itself: the environment is a
parameter. The seed run passes the real environment; the PROFILE renderer
passes a committed canonical environment, so the committed page is a function
of committed bytes rather than of whoever last ran it.
"""

from __future__ import annotations

import datetime as _dt
import hashlib
import json
import os
from collections.abc import Mapping
from pathlib import Path
from typing import Any, TypedDict

from . import config, profiles
from .manifest_sentinel import emit_manifest_sentinel


class PersonaEntry(TypedDict):
    """One roster entry. `fixtures` names the same shape by the role it plays."""

    email: str
    display_name: str
    team: str | None
    role: str
    uuid: str
    org_unit: str
    realm_roles: list[str]


class RealmRef(TypedDict):
    name: str
    issuer: str


class TenantRefs(TypedDict):
    default: str
    other: str


class Capabilities(TypedDict):
    ingestion: bool
    service_principals: bool
    idp: str


class Manifest(TypedDict):
    """The seeded stand's description, as it travels on the wire.

    INVARIANT: `tests/lib/insight_stand/manifest.py` parses this shape. Changing
    the field set is a wire change — bump `MANIFEST_VERSION` with it.
    """

    manifest_version: int
    tenant: str
    tenants: TenantRefs
    realm: RealmRef
    personas: list[PersonaEntry]
    service_urls: dict[str, str]
    fixtures: dict[str, PersonaEntry]
    capabilities: Capabilities
    seed_revision: str
    data_window: str
    anchor_date: str
    seeded: list[str]


MANIFEST_VERSION = 2

# Values shared with `keycloak_realm`, which builds the Keycloak realm from
# this same roster. The two must agree exactly or a persona will authenticate
# as someone the API does not recognise.
REALM_NAME = "insight"  # keycloak_realm.REALM_NAME
EXECUTIVE_ORG_UNIT = "executive"  # keycloak_realm._org_unit, teamless
OPERATOR_ORG_UNIT = "operations"  # keycloak_realm.OPERATOR_ORG_UNIT
ROLE_TO_REALM_ROLES: dict[str, list[str]] = {
    "ceo": ["insight-admin", "insight-lead"],
    "lead": ["insight-lead"],
    "ic": ["insight-member"],
    "admin": ["insight-admin"],
}

# Compose-network-internal addresses: reachable from other containers on the
# `insight` network, never a host-published port. Host-side drivers resolve
# their own URLs (dev-compose.sh urls / the test-stand env file).
SERVICE_URLS: dict[str, str] = {
    "gateway": "http://gateway:8080",
    "identity": "http://identity:8082",
    "analytics": "http://analytics:8081",
    "authenticator": "http://authenticator:8083",
    # The authenticator's SECOND listener, carrying service-to-service issuance
    # only, on its own port so it never shares the browser surface. A caller
    # inside the network uses this; a host-side one uses the published
    # AUTHENTICATOR_TOKEN_PORT.
    #
    # Named `_s2s` rather than `_token` deliberately: `assert_no_credentials`
    # rejects any key containing "token", and it is right to — a manifest is not
    # allowed to carry one. This value is an address, and the guard should not
    # have to distinguish.
    "authenticator_s2s": "http://authenticator:8093",
    "insight_front": "http://insight-front",
    "clickhouse": "http://clickhouse:8123",
    "mariadb": "mariadb:3306",
}

# The environment the COMMITTED PROFILE.md is rendered against. Keeping it
# separate from the real environment is what stops the committed page from
# embedding one developer's DEV_USER_EMAIL or one stand's issuer, which would
# make the staleness check fail for everybody else.
CANONICAL_ENV: dict[str, str] = {
    "DEV_USER_EMAIL": "email_development_lead@company.nonpresent",
    "TENANT_DEFAULT_ID": profiles.TENANT_DEFAULT
    if hasattr(profiles, "TENANT_DEFAULT")
    else "00000000-df51-5b42-9538-d2b56b7ee953",
    "SEED_ANCHOR_DATE": "2026-06-30",
    "SEED_DAYS": "60",
    "AUTHENTICATOR_OIDC_ISSUER": "",
    # The canonical stand carries the cross-tenant refusal fixture, so the
    # committed PROFILE.md describes a compose stand — the one the suite reads.
    config.CROSS_TENANT_FIXTURE_ENV: "1",
}

# Literals that must never reach the manifest. Checked before the file is
# written, so a future field that accidentally carries a secret fails loudly
# instead of shipping.
_STATIC_FORBIDDEN_LITERALS = frozenset(
    {
        "insight-dev",  # keycloak_realm dev password (the default)
        "insight-authenticator-dev-secret",  # keycloak_realm client secret
        "insight-local",  # MariaDB / ClickHouse dev password
        "root-local",  # MariaDB root password
    }
)


def _forbidden_literals(env: Mapping[str, str]) -> frozenset[str]:
    """The dev credentials, plus the persona password when a stand overrides it.

    SAFETY: an empty override is dropped rather than added. The scan is a
    substring test, so `""` matches every document — and it runs after every
    database write, in a Job with `backoffLimit: 0`, where refusing reports a
    correctly seeded stand as a failure that cannot be retried.
    """
    override = (env.get(config.PERSONA_PASSWORD_ENV) or "").strip()
    if not override:
        return _STATIC_FORBIDDEN_LITERALS
    return _STATIC_FORBIDDEN_LITERALS | {override}


_FORBIDDEN_KEY_SUBSTRINGS = ("password", "secret", "token", "credential", "passwd")


def manifest_path() -> Path:
    """Where this run writes its manifest — see `config.parse_manifest_path`."""
    import os

    return config.parse_manifest_path(os.environ)


# The window comes from `config`, the same reader the generators use, so the
# window this document reports and the dates the rows carry cannot disagree —
# two independent `now()` calls straddling UTC midnight is exactly how they
# would.
_anchor = config.parse_anchor_date
_days = config.parse_seed_days


def seed_revision() -> str:
    """Content hash over the seed package's Python sources.

    Identifies the generator code that produced a stand, with no git dependency
    and no clock. Any edit to the package changes it, which is the point: it is
    what makes a committed PROFILE.md detectably stale. Tests are deliberately
    outside the hash — they cannot change what a run writes.
    """
    root = Path(__file__).resolve().parent
    digest = hashlib.sha256()
    for path in sorted(root.rglob("*.py")):
        parts = path.relative_to(root).parts
        if "__pycache__" in parts or any(p.endswith(".egg-info") for p in parts):
            continue
        if ".venv" in parts:
            continue
        digest.update("/".join(parts).encode())
        digest.update(path.read_bytes())
    return digest.hexdigest()[:16]


def _persona(person: profiles.Person) -> PersonaEntry:
    """One roster entry. Fields are named explicitly, never spread from the
    Person object, so a future attribute cannot leak into the document."""
    # Mirrors `keycloak_realm._org_unit`: teamless people are the CEO
    # (executive) and the admin operator (operations, its own unit because it
    # administers the product rather than belonging to the org).
    if person.team is not None:
        org_unit = person.team
    elif person.role == "admin":
        org_unit = OPERATOR_ORG_UNIT
    else:
        org_unit = EXECUTIVE_ORG_UNIT
    return {
        "email": person.email,
        "display_name": f"{person.first_name} {person.last_name}".strip(),
        "team": person.team,
        "role": person.role,
        "uuid": person.uuid,
        "org_unit": org_unit,
        "realm_roles": ROLE_TO_REALM_ROLES[person.role],
    }


def _fixtures(personas: list[PersonaEntry]) -> dict[str, PersonaEntry]:
    """Named catalog the test suite declares its data needs against.

    Names are stable contracts — renaming one breaks every test that declares
    it — so they describe a ROLE in the org, not a person.
    """
    by_uuid = {p["uuid"]: p for p in personas}

    def ref(person: PersonaEntry) -> PersonaEntry:
        return {
            "uuid": person["uuid"],
            "email": person["email"],
            "display_name": person["display_name"],
            "team": person["team"],
            "org_unit": person["org_unit"],
            "role": person["role"],
            "realm_roles": person["realm_roles"],
        }

    catalog: dict[str, PersonaEntry] = {}
    if profiles.DEV_LEAD_UUID in by_uuid:
        catalog["dev_lead"] = ref(by_uuid[profiles.DEV_LEAD_UUID])
    if profiles.CEO_UUID in by_uuid:
        catalog["ceo"] = ref(by_uuid[profiles.CEO_UUID])
    # The admin operator holds the `admin` row in `identity.person_roles`, which
    # is the ONLY thing that opens the admin-gated identity API — a realm role
    # does not. It is deliberately outside the org chart, so a test using it
    # cannot perturb any visibility assertion.
    if profiles.ADMIN_OPERATOR_UUID in by_uuid:
        catalog["admin_operator"] = ref(by_uuid[profiles.ADMIN_OPERATOR_UUID])
    # The second tenant's only person. Named like any other fixture so a
    # cross-tenant test declares `requires_seed("other_tenant_lead")` rather
    # than reaching for a UUID — and so a stand seeded without them says which
    # name is missing.
    if profiles.OTHER_TENANT_PERSON_UUID in by_uuid:
        catalog["other_tenant_lead"] = ref(by_uuid[profiles.OTHER_TENANT_PERSON_UUID])
    for name, uuid in (
        ("sales_lead", profiles.SALES_LEAD_UUID),
        ("hr_lead", profiles.HR_LEAD_UUID),
        ("support_lead", profiles.SUPPORT_LEAD_UUID),
    ):
        if uuid in by_uuid:
            catalog[name] = ref(by_uuid[uuid])
    # One individual contributor per team, so a test can exercise the
    # non-privileged view without hardcoding a UUID.
    for person in personas:
        if person["role"] != "ic" or person["team"] is None:
            continue
        key = f"{person['team']}_ic"
        if key not in catalog:
            catalog[key] = ref(person)
    return catalog


def build_manifest(
    env: Mapping[str, str],
    seeded: list[str] | None = None,
) -> Manifest:
    """Build the manifest document. Pure: no I/O beyond reading own sources."""
    dev_email = (env.get("DEV_USER_EMAIL") or "").strip().lower()
    if not dev_email:
        dev_email = CANONICAL_ENV["DEV_USER_EMAIL"]
    tenant = (env.get("TENANT_DEFAULT_ID") or "").strip() or CANONICAL_ENV["TENANT_DEFAULT_ID"]

    anchor = _anchor(env)
    days = _days(env)
    window_start = anchor - _dt.timedelta(days=days - 1)

    # The second tenant's person appears here only when the seed run actually
    # wrote them (`identity.py` reads the same switch). Advertising a fixture
    # whose row does not exist would turn every test that declares
    # `requires_seed("other_tenant_lead")` from a skip into a failure.
    roster = list(profiles.build_seeded_roster(dev_email, config.parse_org_headcount(env)))
    if config.cross_tenant_fixture_enabled(env):
        roster += profiles.build_other_tenant_roster()

    personas = [_persona(p) for p in roster]

    issuer = (env.get("AUTHENTICATOR_OIDC_ISSUER") or "").strip()
    # Mirrors `profiles.get_idp_source_type`: consumers use this value as the
    # identity source_type of the seeded login rows, so it must be the one
    # they were actually written under.
    idp = (env.get("IDP_SOURCE_TYPE") or "").strip() or "keycloak"

    return {
        "manifest_version": MANIFEST_VERSION,
        "tenant": tenant,
        # `tenant` above stays the default one, unchanged, because everything
        # already reads it. This names BOTH, so a cross-tenant test can say
        # which caller it means without hardcoding a UUID.
        "tenants": {"default": tenant, "other": profiles.TENANT_OTHER},
        "realm": {"name": REALM_NAME, "issuer": issuer},
        "personas": personas,
        "service_urls": dict(SERVICE_URLS),
        "fixtures": _fixtures(personas),
        "capabilities": {
            # Compose seeds silver/gold directly; no connector ever runs, so
            # the ingestion path is not exercised on this stand.
            "ingestion": False,
            "service_principals": config.service_principals_reachable(env),
            "idp": idp,
        },
        "seed_revision": seed_revision(),
        "data_window": f"{window_start.isoformat()}..{anchor.isoformat()}",
        "anchor_date": anchor.isoformat(),
        "seeded": sorted(seeded or []),
    }


def assert_no_credentials(doc: Mapping[str, Any], env: Mapping[str, str] | None = None) -> None:
    """Fail loudly if a secret ever reaches the document."""
    blob = json.dumps(doc, ensure_ascii=False)
    for literal in _forbidden_literals(os.environ if env is None else env):
        if literal in blob:
            raise RuntimeError(
                f"manifest would contain the credential literal {literal!r}; "
                "the manifest carries references, never secrets"
            )

    def walk(node: Any, path: str = "") -> None:
        if isinstance(node, dict):
            for key, value in node.items():
                lowered = str(key).lower()
                if any(bad in lowered for bad in _FORBIDDEN_KEY_SUBSTRINGS):
                    raise RuntimeError(f"manifest field {path}{key!r} looks credential-bearing")
                walk(value, f"{path}{key}.")
        elif isinstance(node, list):
            for item in node:
                walk(item, path)

    walk(doc)


def render_manifest(doc: Manifest) -> str:
    """Serialise deterministically: sorted keys, fixed indent, LF, trailing newline."""
    return json.dumps(doc, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


def write_manifest(doc: Manifest, path: Path | None = None) -> Path:
    # The scrub gate runs before either transport touches `doc`: the sentinel
    # line and the file are two renderings of the one object it already cleared.
    assert_no_credentials(doc)
    emit_manifest_sentinel(doc)
    target = path or manifest_path()
    target.write_text(render_manifest(doc), encoding="utf-8", newline="\n")
    return target
