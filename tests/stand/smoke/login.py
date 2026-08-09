"""Winning a session on a DEPLOYED stand, where the compose credential story does not hold.

`insight_stand.personas.open_session` is the suite's normal way in, and this
module deliberately does NOT use it — for exactly two reasons, both about where
the credential comes from rather than about how the login works:

* `persona_password()` resolves ONE variable (`INSIGHT_STAND_PERSONA_PASSWORD`)
  shared by every persona, and otherwise falls back to
  `deploy/compose/keycloak/realm-insight.generated.json`. That file does not
  exist for a cluster stand, and a deploy gate wants the option of a distinct
  secret per persona.
* A cluster stand may not be able to serve a password form at all (see
  "the login blocker" below), in which case the only honest way in is the
  product's own view-as path. That is a different `/auth/login` request, not a
  different transport.

Everything BELOW the credential is reused verbatim: `LoginSession` drives the
real authorization-code+PKCE chain through the public URL, and `ApiClient` is
the only thing that issues requests. Nothing here mints a token, forges a
cookie, or talks to Keycloak's admin API.


The login blocker
-----------------

A Keycloak realm that brokers login to an external OAuth provider renders no
username/password form: its browser flow is `auth-cookie` OR
`identity-provider-redirector`, so the authorize endpoint answers a redirect
straight to the provider and there is nothing for a script to submit.
`LoginSession._fetch_login_form` fails there by design — it requires a 200
carrying a `login-actions/authenticate` form, and it refuses to post credentials
to any origin but the IdP's. No amount of test code works around that; the realm
has to present a password form, or the login has to be a different login.

So this module implements BOTH supported configurations and makes the operator
choose one explicitly, with `$SMOKE_LOGIN_MODE`:

`password` (default)
    Every persona authenticates as themselves. Requires the stand's realm to
    carry a LOCAL user per persona, with a password supplied by the environment.

`override`
    ONE bootstrap principal authenticates, and each persona session is minted
    through the product's own view-as path — `GET /auth/login?__override=<email>`
    — which the authenticator resolves by email against the same
    `identity.persons` rows the seeder writes. Requires the authenticator to run
    with `override_enabled`.

What is NOT implemented, and will not be: exchanging a client-credentials or
service token for a persona session. The gateway reads `__Host-sid` and
OVERWRITES the `Authorization` header with the JWT it fetches for that session,
so a token minted anywhere else cannot reach `/api/*` through the public URL —
it would test nothing this suite is for.

When neither mode can complete, the failure is loud and names the realm: see
`describe_login_failure`. It is never a skip.
"""

from __future__ import annotations

import os
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from enum import StrEnum
from typing import Final
from urllib.parse import quote, urlsplit

import httpx
from insight_stand import (
    ApiClient,
    LoginNotCompletedError,
    LoginSession,
    Manifest,
    Person,
    PersonaError,
    PersonaSession,
    verify_realm_roles,
)
from insight_stand.session import LOGIN_PATH

#: The stand's PUBLIC address. Required, and deliberately without a default of
#: any kind: this suite is aimed at a deployed stand from the outside, and a
#: default would either be a real host committed to a public repository or a
#: localhost guess that silently smoke-tests the wrong thing.
BASE_URL_ENV: Final[str] = "SMOKE_BASE_URL"

#: `password` | `override` — see the module docstring. Defaulted rather than
#: required because `password` is the configuration we want stands to have; a
#: stand that needs `override` is opting out of that and should say so.
LOGIN_MODE_ENV: Final[str] = "SMOKE_LOGIN_MODE"

#: One secret shared by every persona, which is what a stand provisioned from a
#: single CI secret has.
PERSONA_PASSWORD_ENV: Final[str] = "SMOKE_PERSONA_PASSWORD"

#: `SMOKE_PERSONA_PASSWORD__DEV_LEAD` overrides the shared value for the
#: `dev_lead` manifest fixture. The suffix is the fixture name upper-cased with
#: `-` mapped to `_`, so it is a legal environment-variable name and still reads
#: as the fixture it belongs to.
PERSONA_PASSWORD_PREFIX: Final[str] = "SMOKE_PERSONA_PASSWORD__"

#: `override` mode only: the one principal that can actually authenticate.
BOOTSTRAP_EMAIL_ENV: Final[str] = "SMOKE_BOOTSTRAP_EMAIL"
BOOTSTRAP_PASSWORD_ENV: Final[str] = "SMOKE_BOOTSTRAP_PASSWORD"

#: The view-as query parameter the authenticator reads (`LoginParams.__override`).
#: Honoured only when the deployment sets `override_enabled`; otherwise the
#: authenticator logs the attempt and ignores the parameter, which shows up here
#: as a session belonging to the bootstrap principal rather than the persona —
#: caught by the `/auth/me` check, not papered over.
OVERRIDE_PARAM: Final[str] = "__override"


class LoginMode(StrEnum):
    """How a persona session is obtained on this stand."""

    PASSWORD = "password"
    OVERRIDE = "override"


def _required(environ: Mapping[str, str], name: str, why: str) -> str:
    value = (environ.get(name) or "").strip()
    if not value:
        raise PersonaError(
            f"${name} is not set. {why}\n"
            f"  See tests/stand/smoke/README.md for the full variable list. "
            f"Nothing here has a default — a deploy gate that guesses an address "
            f"or a credential is worse than one that refuses to start."
        )
    return value


@dataclass(frozen=True)
class SmokeCredentials:
    """Everything the smoke needs to authenticate, resolved from the environment.

    The secrets carry `repr=False` for the same reason `LoginSession.password`
    does: pytest prints fixture reprs in a traceback, and this suite's output is
    a public CI log.
    """

    mode: LoginMode
    base_url: str
    bootstrap_email: str = ""
    bootstrap_password: str = field(default="", repr=False)
    shared_password: str = field(default="", repr=False)
    persona_passwords: Mapping[str, str] = field(default_factory=dict, repr=False)

    def password_for(self, fixture_name: str) -> str:
        """The credential this persona logs in with, per-persona value first."""
        specific = self.persona_passwords.get(fixture_name, "")
        if specific:
            return specific
        if self.shared_password:
            return self.shared_password
        raise PersonaError(
            f"no password for the {fixture_name!r} persona: neither "
            f"${PERSONA_PASSWORD_PREFIX}{_env_suffix(fixture_name)} nor "
            f"${PERSONA_PASSWORD_ENV} is set"
        )


def _env_suffix(fixture_name: str) -> str:
    return fixture_name.upper().replace("-", "_")


def resolve_credentials(environ: Mapping[str, str] | None = None) -> SmokeCredentials:
    """Read the smoke's configuration, or raise `PersonaError` naming what is missing.

    Raising beats defaulting at every branch here. A missing base URL, a missing
    password and an unknown mode are all operator mistakes that a deploy gate
    must report before it touches the stand, not discover halfway through a
    parametrized run.
    """
    env = os.environ if environ is None else environ

    base_url = _required(
        env,
        BASE_URL_ENV,
        "It is the stand's public address, the one a human would type.",
    ).rstrip("/")

    raw_mode = (env.get(LOGIN_MODE_ENV) or LoginMode.PASSWORD.value).strip().lower()
    try:
        mode = LoginMode(raw_mode)
    except ValueError:
        modes = ", ".join(sorted(m.value for m in LoginMode))
        raise PersonaError(
            f"${LOGIN_MODE_ENV}={raw_mode!r} is not a login mode; use one of: {modes}. "
            "See tests/stand/smoke/README.md for what each one requires of the stand."
        ) from None

    per_persona = {
        key[len(PERSONA_PASSWORD_PREFIX) :].lower(): value.strip()
        for key, value in env.items()
        if key.startswith(PERSONA_PASSWORD_PREFIX) and value.strip()
    }

    if mode is LoginMode.OVERRIDE:
        return SmokeCredentials(
            mode=mode,
            base_url=base_url,
            bootstrap_email=_required(
                env,
                BOOTSTRAP_EMAIL_ENV,
                f"In {LoginMode.OVERRIDE.value!r} mode one principal authenticates for real "
                "and every persona session is minted from it.",
            ),
            bootstrap_password=_required(
                env,
                BOOTSTRAP_PASSWORD_ENV,
                f"It is the bootstrap principal's IdP password ({BOOTSTRAP_EMAIL_ENV}).",
            ),
        )

    return SmokeCredentials(
        mode=mode,
        base_url=base_url,
        shared_password=_required(
            env,
            PERSONA_PASSWORD_ENV,
            f"In {LoginMode.PASSWORD.value!r} mode every persona authenticates as themselves. "
            f"One shared value is enough; ${PERSONA_PASSWORD_PREFIX}<FIXTURE> overrides it "
            "for a single persona.",
        ),
        persona_passwords=per_persona,
    )


def override_login_path(email: str) -> str:
    """`/auth/login?__override=<email>` — the product's own view-as entry point."""
    return f"{LOGIN_PATH}?{OVERRIDE_PARAM}={quote(email, safe='@')}"


def _redact(url: str | None) -> str:
    """Scheme, host and path — never the query string.

    An IdP authorize URL carries `state`, `nonce`, `code_challenge` and the
    client id, and everything this module writes lands in a CI log on a public
    repository. The path alone is what makes a failure diagnosable; the query is
    only noise that happens to be sensitive.
    """
    if not url:
        return "<unknown>"
    parts = urlsplit(url)
    if not parts.scheme or not parts.netloc:
        return parts.path or "<unknown>"
    return f"{parts.scheme}://{parts.netloc}{parts.path}"


def describe_login_failure(
    *,
    name: str,
    email: str,
    manifest: Manifest,
    credentials: SmokeCredentials,
    reason: str,
    stopped_at: str | None,
) -> str:
    """The actionable message a failed login reports, with the realm named.

    Long on purpose. The overwhelmingly likely cause is a stand-configuration
    fact — the realm has no password form, or view-as is switched off — and
    whoever reads this in a CI log is not the person who wrote the suite. A
    one-line "login failed" would send them to the wrong place every time.
    """
    realm = manifest.realm
    issuer = _redact(realm.issuer) if realm.issuer else "<not recorded by the seed>"
    return "\n".join(
        [
            f"persona {name!r} ({email}) could not obtain a session at "
            f"{credentials.base_url} in {credentials.mode.value!r} mode.",
            f"  stopped at: {_redact(stopped_at)}",
            f"  reason:     {reason}",
            f"  realm:      {realm.name!r} (issuer {issuer}), per {manifest.source_path}",
            "",
            "The usual cause is the stand's realm, not the product. A Keycloak realm",
            "that brokers login to an external OAuth provider renders NO username/password",
            "form — its browser flow redirects straight to the provider — so a scripted",
            "login has nothing to submit. insight_stand's LoginSession stops there",
            "deliberately rather than inventing a credential path the product does not have.",
            "",
            "Two supported configurations, and this suite implements both:",
            "",
            f"  {LOGIN_MODE_ENV}={LoginMode.PASSWORD.value}",
            "      The stand's realm carries a LOCAL user per persona, with a password form",
            f"      in its browser flow. Credentials come from ${PERSONA_PASSWORD_ENV}",
            f"      (or ${PERSONA_PASSWORD_PREFIX}<FIXTURE> per persona).",
            "",
            f"  {LOGIN_MODE_ENV}={LoginMode.OVERRIDE.value}",
            "      The authenticator runs with override_enabled, ONE principal authenticates",
            f"      (${BOOTSTRAP_EMAIL_ENV} / ${BOOTSTRAP_PASSWORD_ENV}), and every persona",
            "      session is minted through GET /auth/login?__override=<email>, which",
            "      resolves by email against the persons rows the seeder writes.",
            "",
            "Neither is something a test can arrange for itself: both are stand",
            "configuration. tests/stand/smoke/README.md states what has to be true.",
        ]
    )


@dataclass(frozen=True)
class SmokeLogin:
    """One persona's login attempt — the session, or why there is none.

    A failed login is CARRIED rather than raised so the login check can report
    it as a failing assertion naming the persona, instead of every test that
    touches that persona erroring out in fixture setup with the same traceback.
    """

    name: str
    person: Person
    mode: LoginMode
    persona: PersonaSession | None = None
    failure: str | None = None

    def require(self) -> PersonaSession:
        """The session, or an `AssertionError` carrying the whole diagnosis.

        The only accessor, deliberately: an `ok` predicate beside it would invite
        `assert attempt.ok, attempt.failure`, and pytest's assertion rewriting
        then appends a truncated `SmokeLogin(...)` repr underneath the sentence
        that was supposed to be the whole message.
        """
        if self.persona is None:
            raise AssertionError(self.failure or f"no session for persona {self.name!r}")
        return self.persona


def open_smoke_session(
    name: str,
    manifest: Manifest,
    credentials: SmokeCredentials,
    *,
    timeout_s: float = 30.0,
) -> SmokeLogin:
    """Log a manifest fixture in, capturing any failure instead of raising it.

    `name` is a key in the manifest's `fixtures{}` catalog — never an email and
    never a UUID — so a roster reshuffle moves the person without touching a
    test. The email that reaches the IdP (and, in override mode, the email in
    the `__override` parameter) is read off that fixture.
    """
    person = manifest.fixture(name)

    try:
        # Cheap, and it is the one thing that catches a seed whose roster and
        # role mapping disagree. On a cluster the realm export is absent, so
        # only the manifest-vs-roster half runs — which is the half that would
        # otherwise let a persona carry quietly wrong authority.
        verify_realm_roles(person)
        password = (
            credentials.bootstrap_password
            if credentials.mode is LoginMode.OVERRIDE
            else credentials.password_for(name)
        )
    except PersonaError as exc:
        return SmokeLogin(
            name=name,
            person=person,
            mode=credentials.mode,
            failure=(
                f"persona {name!r} ({person.email}) is not usable on this stand "
                f"before any login was attempted: {exc}"
            ),
        )

    if credentials.mode is LoginMode.OVERRIDE:
        session = LoginSession(
            base_url=credentials.base_url,
            email=credentials.bootstrap_email,
            password=password,
            login_path=override_login_path(person.email),
            timeout_s=timeout_s,
        )
    else:
        session = LoginSession(
            base_url=credentials.base_url,
            email=person.email,
            password=password,
            timeout_s=timeout_s,
        )

    try:
        session.login()
    except LoginNotCompletedError as exc:
        return SmokeLogin(
            name=name,
            person=person,
            mode=credentials.mode,
            failure=describe_login_failure(
                name=name,
                email=person.email,
                manifest=manifest,
                credentials=credentials,
                reason=str(exc),
                stopped_at=exc.stopped_at,
            ),
        )
    except httpx.HTTPError as exc:
        # A transport failure part-way through the chain — DNS, TLS, a timeout,
        # a reset. Distinguished from the protocol failure above because it says
        # something completely different about the deployment, and because the
        # long realm diagnosis would be actively misleading here.
        return SmokeLogin(
            name=name,
            person=person,
            mode=credentials.mode,
            failure=(
                f"the login chain for persona {name!r} ({person.email}) did not complete "
                f"because a request failed in transport: {type(exc).__name__}: {exc}\n"
                f"  stand: {credentials.base_url}\n"
                f"  This is the stand or the network, not the realm — the IdP hop leaves "
                f"this runner, so a stand reachable at its own address can still fail here "
                f"when the IdP's hostname does not resolve from CI."
            ),
        )

    return SmokeLogin(
        name=name,
        person=person,
        mode=credentials.mode,
        persona=PersonaSession(
            name=name,
            person=person,
            session=session,
            client=ApiClient(base_url=credentials.base_url, session=session, timeout_s=timeout_s),
        ),
    )


__all__: Sequence[str] = (
    "BASE_URL_ENV",
    "BOOTSTRAP_EMAIL_ENV",
    "BOOTSTRAP_PASSWORD_ENV",
    "LOGIN_MODE_ENV",
    "OVERRIDE_PARAM",
    "PERSONA_PASSWORD_ENV",
    "PERSONA_PASSWORD_PREFIX",
    "LoginMode",
    "SmokeCredentials",
    "SmokeLogin",
    "describe_login_failure",
    "open_smoke_session",
    "override_login_path",
    "resolve_credentials",
)
