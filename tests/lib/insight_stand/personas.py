"""Turning a manifest fixture name into a logged-in identity.

Three things live here, all in service of one guarantee: when a test says
`session_for("dev_lead")`, the session it gets back really belongs to that person
and really carries the authority the roster says they have.

* **Credential sourcing.** Never a literal in this tree, and never from the
  manifest — the manifest names who to log in as and deliberately carries no
  secrets.
* **Role verification.** The roster says what each role should be granted; the
  stand says what it actually granted. A mismatch fails the fixture loudly
  rather than producing a session with quietly wrong authority.
* **`PersonaSession`.** The person, their session and a ready client.
"""

from __future__ import annotations

import json
import os
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Final

from .api import ApiClient
from .errors import PersonaError
from .manifest import Manifest, Person
from .session import LoginSession

_REPO_ROOT: Final[Path] = Path(__file__).resolve().parents[3]

# The realm the stand actually imported. Generated per stand and gitignored —
# runtime state describing one stand, exactly like the seed manifest. It is the
# only place that knows what Keycloak was told about these users.
REALM_EXPORT_PATH: Final[Path] = (
    _REPO_ROOT / "deploy" / "compose" / "keycloak" / "realm-insight.generated.json"
)

# Set this to supply the persona password from a secret store / CI secret. It
# takes precedence over the realm export.
PASSWORD_ENV: Final[str] = "INSIGHT_STAND_PERSONA_PASSWORD"

# Mirrors `_ROLE_TO_REALM_ROLES` in deploy/compose/keycloak/gen-realm.py, which
# is what actually builds the realm. Duplicated rather than imported because
# that module belongs to the seed/compose tree; `verify_realm_roles` below is
# what keeps the copy honest, by checking it against the realm the stand ran.
ROLE_TO_REALM_ROLES: Final[Mapping[str, tuple[str, ...]]] = {
    "ceo": ("insight-admin", "insight-lead"),
    "lead": ("insight-lead",),
    "ic": ("insight-member",),
    # The admin operator. Its realm role is NOT what grants it administrative
    # authority — see `OPERATOR_PERSON_ROLE`.
    "admin": ("insight-admin",),
}

# Which realm role makes a persona eligible for each role-aware fixture.
ADMIN_ROLE: Final[str] = "insight-admin"
LEAD_ROLE: Final[str] = "insight-lead"
MEMBER_ROLE: Final[str] = "insight-member"

#: Roster `role` of an account that administers the product instead of
#: belonging to the organisation it measures. Such a person has no team and no
#: place in the org chart; the seed grants it the `admin` row in
#: `identity.person_roles`, which is the only thing that opens the admin-gated
#: identity API.
OPERATOR_PERSON_ROLE: Final[str] = "admin"

#: The manifest fixture name for that account.
ADMIN_OPERATOR_FIXTURE: Final[str] = "admin_operator"

#: The manifest fixture name for the second tenant's only person. Their whole
#: purpose is to be a caller the product refuses, so they hold no role, no team
#: and no org-chart edge — see `deploy/seed/profiles.py::build_other_tenant_roster`.
OTHER_TENANT_FIXTURE: Final[str] = "other_tenant_lead"


def _load_realm(path: Path | None = None) -> dict[str, Any] | None:
    """The stand's realm export, or None when it is not on disk.

    Absence is not an error here: a stand reached over the network has no local
    realm file, and `PASSWORD_ENV` covers that case.
    """
    target = path or REALM_EXPORT_PATH
    try:
        raw = target.read_text(encoding="utf-8")
    except OSError:
        return None
    try:
        doc = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise PersonaError(f"{target} is not valid JSON: {exc}") from exc
    return doc if isinstance(doc, dict) else None


def _realm_user(email: str, realm: Mapping[str, Any]) -> Mapping[str, Any] | None:
    wanted = email.strip().lower()
    for user in realm.get("users", []):
        if not isinstance(user, Mapping):
            continue
        if str(user.get("username", "")).strip().lower() == wanted:
            return user
        if str(user.get("email", "")).strip().lower() == wanted:
            return user
    return None


def persona_password(email: str, *, realm_path: Path | None = None) -> str:
    """The password to log this persona in with.

    Resolution order, both sourced and neither a literal in this tree:

    1. `INSIGHT_STAND_PERSONA_PASSWORD` — the secret-store / CI path, and the
       only option for a stand whose realm this checkout cannot see.
    2. The stand's own generated realm export. It is the authority on what
       Keycloak was told, so reading it means the suite follows a realm change
       instead of drifting from it.

    Raises `PersonaError` rather than guessing.
    """
    supplied = (os.environ.get(PASSWORD_ENV) or "").strip()
    if supplied:
        return supplied

    realm = _load_realm(realm_path)
    if realm is None:
        raise PersonaError(
            f"no password for {email!r}: ${PASSWORD_ENV} is unset and no realm export at "
            f"{realm_path or REALM_EXPORT_PATH}. Bring a stand up "
            f"(./dev-compose.sh test-stand up), which generates it, or export "
            f"{PASSWORD_ENV}."
        )

    user = _realm_user(email, realm)
    if user is None:
        raise PersonaError(f"{email!r} is not a user in {realm_path or REALM_EXPORT_PATH}")
    for credential in user.get("credentials", []):
        if isinstance(credential, Mapping) and credential.get("type") == "password":
            value = str(credential.get("value") or "")
            if value:
                return value
    raise PersonaError(
        f"{email!r} has no password credential in {realm_path or REALM_EXPORT_PATH}; "
        f"export {PASSWORD_ENV} instead"
    )


def expected_realm_roles(person: Person) -> tuple[str, ...]:
    """What the roster says this person's role should be granted."""
    try:
        return ROLE_TO_REALM_ROLES[person.role]
    except KeyError:
        known = ", ".join(sorted(ROLE_TO_REALM_ROLES))
        raise PersonaError(
            f"{person.email!r} has roster role {person.role!r}, which maps to no realm "
            f"roles; known roles: {known}"
        ) from None


def verify_realm_roles(person: Person, *, realm_path: Path | None = None) -> None:
    """Fail unless the roster, the manifest and the realm all agree.

    Two independent checks, because they catch different faults:

    * manifest vs roster — the seed wrote roles that do not follow its own
      role mapping.
    * realm vs roster — Keycloak was told something different from what the
      seed recorded, so the session will carry authority the test does not
      expect. Skipped only when the realm export is not on disk.

    Either mismatch raises. A warning would leave a visibility assertion
    passing for the wrong reason, which is worse than no assertion at all.
    """
    expected = set(expected_realm_roles(person))

    declared = set(person.realm_roles)
    if declared != expected:
        raise PersonaError(
            f"{person.email!r} ({person.role}) is declared with realm roles "
            f"{sorted(declared)} in the manifest, but the roster maps {person.role!r} to "
            f"{sorted(expected)}"
        )

    realm = _load_realm(realm_path)
    if realm is None:
        return
    user = _realm_user(person.email, realm)
    if user is None:
        raise PersonaError(
            f"{person.email!r} is in the manifest but not in the realm "
            f"{realm_path or REALM_EXPORT_PATH} — the stand cannot log them in"
        )
    granted = {str(role) for role in user.get("realmRoles", [])}
    if granted != expected:
        raise PersonaError(
            f"{person.email!r} ({person.role}) is granted {sorted(granted)} by the realm, "
            f"but the roster maps {person.role!r} to {sorted(expected)}"
        )


@dataclass(frozen=True)
class PersonaSession:
    """A logged-in persona: who they are, their session, and a ready client."""

    name: str
    person: Person
    session: LoginSession
    client: ApiClient

    @property
    def email(self) -> str:
        return self.person.email

    @property
    def password(self) -> str:
        """The credential this persona logged in with.

        Exposed for the browser journeys, which have to type it into the IdP's
        form themselves — reusing it guarantees the API session and the browser
        session authenticate as the same human with the same secret.
        """
        return self.session.password

    @property
    def realm_roles(self) -> tuple[str, ...]:
        return self.person.realm_roles

    def has_realm_role(self, role: str) -> bool:
        return role in self.person.realm_roles


def open_session(
    name: str,
    manifest: Manifest,
    base_url: str,
    *,
    realm_path: Path | None = None,
    timeout_s: float = 30.0,
) -> PersonaSession:
    """Resolve a fixture name to a real, verified, logged-in session.

    `name` is a key in the manifest's `fixtures{}` catalog — never an email or
    a UUID, so a roster change moves the person without touching the tests.
    """
    person = manifest.fixture(name)
    verify_realm_roles(person, realm_path=realm_path)
    session = LoginSession(
        base_url=base_url,
        email=person.email,
        password=persona_password(person.email, realm_path=realm_path),
        timeout_s=timeout_s,
    )
    session.login()
    return PersonaSession(
        name=name,
        person=person,
        session=session,
        client=ApiClient(base_url=base_url, session=session, timeout_s=timeout_s),
    )


def resolve_by_realm_role(manifest: Manifest, role: str, *, excluding: str | None = None) -> str:
    """Pick the fixture name of an ORG MEMBER holding a realm role.

    Chosen by the role the persona actually holds rather than by a hardcoded
    name, so `lead` keeps meaning "somebody the realm made a lead" even if the
    roster is reshuffled. Sorted iteration keeps the choice deterministic.

    Operator accounts are skipped, and that exclusion is load-bearing. The admin
    operator holds `insight-admin` and its fixture name sorts first, so without
    this it would win every admin lookup — and being outside the org chart it
    sees nobody, which would make an admin-vs-lead visibility comparison pass
    while proving nothing. This function answers "which member of the
    organisation holds this role"; the operator is asked for by fixture name.
    """
    for fixture_name in sorted(manifest.fixtures):
        person = manifest.fixtures[fixture_name]
        if person.role == OPERATOR_PERSON_ROLE:
            continue
        roles = set(person.realm_roles)
        if role in roles and (excluding is None or excluding not in roles):
            return fixture_name
    detail = f" (excluding holders of {excluding!r})" if excluding else ""
    raise PersonaError(
        f"no seeded persona holds the realm role {role!r}{detail}; "
        f"available: {', '.join(sorted(manifest.fixtures))}"
    )


__all__: Sequence[str] = (
    "ADMIN_OPERATOR_FIXTURE",
    "ADMIN_ROLE",
    "LEAD_ROLE",
    "MEMBER_ROLE",
    "OPERATOR_PERSON_ROLE",
    "OTHER_TENANT_FIXTURE",
    "PASSWORD_ENV",
    "REALM_EXPORT_PATH",
    "ROLE_TO_REALM_ROLES",
    "PersonaSession",
    "expected_realm_roles",
    "open_session",
    "persona_password",
    "resolve_by_realm_role",
    "verify_realm_roles",
)
