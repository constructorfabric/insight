"""Winning a session on a DEPLOYED stand, where the compose credential story does not hold.

`insight_stand.personas.open_session` is the suite's normal way in, and this
module deliberately does NOT use it — for exactly two reasons, both about where
the credential comes from rather than about how the login works:

* `persona_password()` resolves ONE variable (`INSIGHT_STAND_PERSONA_PASSWORD`)
  shared by every persona, and otherwise falls back to
  `deploy/compose/keycloak/realm-insight.generated.json`. That file does not
  exist for a cluster stand: a deployed stand's realm is applied to the
  umbrella's bundled Keycloak and never lands in this checkout.
* Not every cluster stand serves a password form (see "two realm shapes"
  below), and the one that does not needs a different `/auth/login` request —
  not a different transport.

Everything BELOW the credential is reused verbatim: `LoginSession` drives the
real authorization-code+PKCE chain through the public URL, and `ApiClient` is
the only thing that issues requests. Nothing here mints a token, forges a
cookie, or talks to Keycloak's admin API.


Two realm shapes, and why both modes stay
-----------------------------------------

A Keycloak realm that brokers login to an external OAuth provider renders no
username/password form: its browser flow is `auth-cookie` OR
`identity-provider-redirector`, so the authorize endpoint answers a redirect
straight to the provider and there is nothing for a script to submit.
`LoginSession._fetch_login_form` fails there by design — it requires a 200
carrying a `login-actions/authenticate` form, and it refuses to post credentials
to any origin but the IdP's. That is a fact about Keycloak, and no amount of
test code works around it.

**The test stand is not that shape.** Its realm is `insight`, generated from the
SEEDER'S OWN ROSTER by the deployment repository's deploy step, so every user is
a local account with a password credential and a matching row in
`identity.persons`. It serves a real form, and the chain `LoginSession` already
implements is exactly the chain it serves — verified end to end against the
deployed stand, with no change to any step. `password` is therefore the default
here and the mode this stand runs.

`override` stays anyway, and the argument is concrete rather than defensive: the
deployment repository's DEFAULT login mode is the federated one. On a stand
deployed that way `password` cannot complete by construction, and `override` is
the only way this gate runs there at all — deleting it would narrow the gate to
one stand shape. The counterweight belongs in the same breath: THIS stand does
not use it, so nothing in this suite depends on the authenticator running with
`override_enabled`, and turning that flag off cannot break the smoke.

So the two modes, chosen explicitly with `$SMOKE_LOGIN_MODE`:

`password` (default)
    Every persona authenticates as themselves. Requires the stand's realm to
    carry a LOCAL user per persona, with a password supplied by the environment.
    On a realm the seeder generated, that password is ONE constant shared by
    every user — `DEV_PASSWORD` in
    `src/ingestion/tools/seed/insight_seed/keycloak_realm.py`. The suite never
    reads it: the seed manifest refuses to carry that literal, so the value can
    only arrive from the environment. tests/stand/smoke/README.md says where an
    operator gets it and why the CI secret holding it is plumbing rather than a
    cryptographic boundary.

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

When a login cannot complete, the failure is loud and names the step it stopped
at: see `describe_login_failure`, which diagnoses from that step rather than
offering the same essay every time. It is never a skip.
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
from insight_stand.session import CALLBACK_PATH, LOGIN_PATH

#: The stand's PUBLIC address. Required, and deliberately without a default of
#: any kind: this suite is aimed at a deployed stand from the outside, and a
#: default would either be a real host committed to a public repository or a
#: localhost guess that silently smoke-tests the wrong thing.
BASE_URL_ENV: Final[str] = "SMOKE_BASE_URL"

#: `password` | `override` — see the module docstring. Defaulted rather than
#: required because `password` is the configuration we want stands to have, and
#: the configuration the test stand's seeded roster realm actually serves; a
#: stand that needs `override` is a federated one and should say so.
LOGIN_MODE_ENV: Final[str] = "SMOKE_LOGIN_MODE"

#: One value shared by every persona. On a realm the seeder generated that is
#: not a simplification but the truth: the generator writes a single constant as
#: the password credential of every user it emits, so there is nothing else it
#: could be. Sourced from the environment and never from the manifest — the
#: seeder's manifest writer refuses to emit that literal at all.
PERSONA_PASSWORD_ENV: Final[str] = "SMOKE_PERSONA_PASSWORD"

#: `SMOKE_PERSONA_PASSWORD__DEV_LEAD` overrides the shared value for the
#: `dev_lead` manifest fixture. The suffix is the fixture name upper-cased with
#: `-` mapped to `_`, so it is a legal environment-variable name and still reads
#: as the fixture it belongs to.
#:
#: It can express nothing on a seeder-generated realm, where all the users carry
#: one password. Kept for a realm provisioned some other way — a stand whose
#: users were created per-persona with distinct credentials is exactly the case
#: this exists for — and NOT something to reach for on this stand.
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
            "One shared value is what a seeder-generated realm has, because the generator "
            "writes one constant as every user's password — read it out of the checkout "
            "(DEV_PASSWORD in src/ingestion/tools/seed/insight_seed/keycloak_realm.py) "
            "rather than pasting a literal, and see tests/stand/smoke/README.md for why "
            "that is the only place it can come from.",
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


#: The Keycloak endpoint a rendered login form posts to. Its presence in the URL
#: the chain stopped at is what says "the realm DID serve a form and the
#: credential was refused", which is a completely different diagnosis from "no
#: form was ever rendered". `LoginSession._submit_credentials` reports that URL
#: as `stopped_at`, so this is a fact about the flow rather than a guess.
_FORM_SUBMIT_MARKER: Final[str] = "login-actions/authenticate"


def _stopped_at_path(stopped_at: str | None) -> str:
    """The path of the URL the chain stopped at, or `""` when there is none.

    Path only, deliberately: the branch below has to hold for any stand, and the
    host differs per deployment — on a stand that publishes Keycloak under a path
    on the product's own host, the IdP and the product are the SAME origin, so
    the netloc says nothing about which of them answered.
    """
    return urlsplit(stopped_at).path if stopped_at else ""


def _diagnosis(credentials: SmokeCredentials, stopped_at: str | None) -> list[str]:
    """The likely causes, chosen by WHERE the chain stopped.

    An unconditional explanation is worse than none: it is read as a diagnosis,
    and this suite's most likely failure has moved. A stand whose realm the
    seeder generated serves a password form, so "the realm brokers to an external
    provider" — true of a federated stand, and the reason `override` exists — is
    exactly the wrong first place to send a reader who has just mistyped a
    password. Each branch below leads with what can actually have happened at
    that step.
    """
    path = _stopped_at_path(stopped_at)

    if _FORM_SUBMIT_MARKER in path:
        if credentials.mode is LoginMode.OVERRIDE:
            # The persona's email never reached the form in this mode — it went
            # in the query string — so naming the persona variable here would
            # send the reader to a credential this run did not submit.
            return [
                "The realm DID serve a login form and the IdP refused what was submitted, so",
                "this is a credential, not a realm shape. What was submitted is the BOOTSTRAP",
                "principal's, not this persona's. In order of likelihood:",
                "",
                f"  1. ${BOOTSTRAP_PASSWORD_ENV} is not the password the realm holds for",
                f"     ${BOOTSTRAP_EMAIL_ENV}.",
                f"  2. ${BOOTSTRAP_EMAIL_ENV} is not a user of this realm at all.",
                "  3. The realm was re-applied and the bootstrap principal did not survive it.",
            ]
        return [
            "The realm DID serve a login form and the IdP refused what was submitted, so",
            "this is a credential, not a realm shape. In order of likelihood:",
            "",
            f"  1. ${PERSONA_PASSWORD_ENV} does not match the realm's password. On a stand",
            "     whose realm the seeder generated, every user carries ONE constant —",
            "     DEV_PASSWORD in src/ingestion/tools/seed/insight_seed/keycloak_realm.py.",
            "     Read it out of the checkout; the seed manifest deliberately does not",
            "     carry it. tests/stand/smoke/README.md has the one-liner.",
            "  2. The realm was re-applied from a roster this manifest does not describe,",
            "     so the username submitted is not a user on the stand.",
            f"  3. A per-persona override (${PERSONA_PASSWORD_PREFIX}<FIXTURE>) is set and",
            "     is wrong. A seeder-generated realm has no per-user password for it to",
            "     express, so on such a stand the variable can only do harm.",
        ]

    if path == CALLBACK_PATH:
        return [
            "The IdP authenticated the persona and the PRODUCT refused the session, so",
            "Keycloak is fine and identity resolution is not. In order of likelihood:",
            "",
            "  1. The stand was deployed but never seeded, or was re-deployed after being",
            "     seeded. A login authenticates against Keycloak but resolves against the",
            "     identity.persons rows the SEEDER writes; an unseeded stand serves the",
            "     form, accepts the password, and then denies the callback. That is the",
            "     documented sequence, not a defect — seed it and run this again.",
            "  2. The release's external-id claim and the claim the realm actually emits",
            "     disagree. The roster UUID travels as its own claim from a user attribute,",
            "     NOT as `sub`: the chart applies realms through the admin REST API, which",
            "     assigns its own user ids and discards the document's.",
            "  3. That user attribute was dropped on import. Keycloak 26 runs a declarative",
            "     user profile whose default policy discards undeclared attributes, which",
            "     empties the claim while the protocol mappers import perfectly.",
            "  4. The identity source_type the seeder wrote and the one the release resolves",
            "     under are not the same string.",
        ]

    return [
        "The chain stopped before a login form was ever submitted, which points at the",
        "realm's shape or at the edge in front of it:",
        "",
        "  1. The realm brokers login to an external OAuth provider. Such a realm renders",
        "     NO username/password form — its browser flow redirects straight to the",
        "     provider — so a scripted login has nothing to submit. LoginSession stops",
        "     there deliberately rather than inventing a credential path the product does",
        f"     not have. That stand shape is what {LOGIN_MODE_ENV}={LoginMode.OVERRIDE.value}"
        " is for.",
        "  2. The edge does not route /auth/* to the authenticator, or the authenticator",
        "     has no issuer configured for this host — check 1 covers both and would",
        "     already have failed.",
        "  3. The realm serves a form that posts somewhere other than the IdP's own",
        "     origin. LoginSession refuses to send credentials there, on purpose.",
    ]


def describe_login_failure(
    *,
    name: str,
    email: str,
    manifest: Manifest,
    credentials: SmokeCredentials,
    reason: str,
    stopped_at: str | None,
) -> str:
    """The actionable message a failed login reports, diagnosed from where it stopped.

    Long on purpose. The likely cause is almost always a stand-configuration
    fact, and whoever reads this in a CI log is not the person who wrote the
    suite — a one-line "login failed" would send them to the wrong place every
    time. But the causes differ completely by step, so the explanation is chosen
    rather than recited; see `_diagnosis`.
    """
    realm = manifest.realm
    # The seed records an issuer only when the stand told it one, and `Realm`
    # never guesses. Rather than print a bare "<unknown>", point at the thing
    # that is authoritative when the manifest is silent: the issuer the release
    # configured the authenticator with, which is also what check 1 watched the
    # login redirect being built from.
    issuer = (
        _redact(realm.issuer)
        if realm.issuer
        else "not recorded by the seed — compare the authenticator's configured issuerUrl"
    )
    lines = [
        f"persona {name!r} ({email}) could not obtain a session at "
        f"{credentials.base_url} in {credentials.mode.value!r} mode.",
        f"  stopped at: {_redact(stopped_at)}",
        f"  reason:     {reason}",
        f"  realm:      {realm.name!r} (issuer {issuer}), per {manifest.source_path}",
        "",
        *_diagnosis(credentials, stopped_at),
    ]

    if credentials.mode is LoginMode.OVERRIDE:
        lines += [
            "",
            f"One more, because this run is in {LoginMode.OVERRIDE.value!r} mode: the persona's",
            f"email travelled in the {OVERRIDE_PARAM} parameter rather than the login form, and",
            "the authenticator honours that parameter only when the deployment runs with",
            "override_enabled. With it off the parameter is ignored and logged, which shows",
            "up as a session belonging to the bootstrap principal — caught by the /auth/me",
            "check rather than here, but worth ruling out while you are looking.",
        ]

    lines += [
        "",
        "None of these is something a test can arrange for itself: all of them are stand",
        "configuration. tests/stand/smoke/README.md states what has to be true.",
    ]
    return "\n".join(lines)


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
        # step-by-step diagnosis would be actively misleading here.
        #
        # The hint stops short of naming the IdP's origin, because the two
        # deployments this suite serves disagree about it: the test stand
        # publishes Keycloak under a path on the PRODUCT'S OWN host, so every hop
        # is the same origin and a resolvable stand cannot fail at the IdP for
        # DNS reasons; a federated stand's provider is somewhere else entirely
        # and can. Asserting either shape here would be wrong half the time.
        return SmokeLogin(
            name=name,
            person=person,
            mode=credentials.mode,
            failure=(
                f"the login chain for persona {name!r} ({person.email}) did not complete "
                f"because a request failed in transport: {type(exc).__name__}: {exc}\n"
                f"  stand: {credentials.base_url}\n"
                f"  This is the network or a host, not the realm. The chain leaves this "
                f"runner at every hop, and not every hop is necessarily the stand's own "
                f"origin — an IdP published elsewhere has to resolve from CI too."
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
