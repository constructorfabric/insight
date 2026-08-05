"""A service principal on the stand — obtained, not minted.

Some routes are reachable only to a SERVICE, not to a person: identity's
`/internal/*` lookups are the ones this suite cares about, and the authenticator
itself is their only real caller. Proving they refuse a human and serve a
service needs a token whose `sub_type` is `service`, which no login produces.

The in-process rig solves that by signing one. That proves the verifier compares
a claim and nothing about whether such a token can be obtained. Here it is
obtained the way the authenticator's own clients obtain one — RFC 7523
`private_key_jwt`, exchanged at the deployed token endpoint:

    1. sign a short-lived assertion with the `testclient` private key that
       `dev-compose.sh ensure_service_token_dev_key` generates per checkout,
    2. POST it as `client_credentials` to the authenticator's token listener,
    3. receive a normal gateway JWT with `sub_type=service`.

Only step 1 is cryptography, and it is a REQUEST for a token rather than a
token. Steps 2 and 3 are the product's.

**Why this does not go through `ApiClient`.** That client speaks to the gateway
and refuses anything else, which is right: every product surface is behind the
edge. The token listener is not a product surface — it is a second listener on
its own port that services reach in-network, never through the edge — so this
acquires the credential with its own client, exactly as `LoginSession` drives
the browser login chain with its own. What the credential is then used ON still
goes through `ApiClient`.

**Two addresses, and they are not the same one.** The URL this POSTs to depends
on where the runner is (a published host port, or the in-network name when the
suite runs inside the ui-tests image), while the assertion's `aud` must equal
the audience the authenticator was CONFIGURED with — one fixed value it compares
against, whoever is calling. Sending the request URL as the audience works from
inside the network and fails from the host, which is the kind of bug that looks
like a key problem.
"""

from __future__ import annotations

import time
import uuid
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Final

from .errors import PersonaError
from .stand import CANDIDATE_ENV_FILES, PUBLISHED_HOST, parse_env_file

_REPO_ROOT: Final[Path] = Path(__file__).resolve().parents[3]

#: RFC 7523 assertion type — the only one the authenticator accepts.
ASSERTION_TYPE: Final[str] = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer"

#: The dev service registered in the authenticator's config (`service_tokens.
#: services`). DEV/TEST ONLY: no key material is committed, `dev-compose.sh`
#: generates the pair per checkout and mounts the public half.
SERVICE_NAME: Final[str] = "testclient"

#: Private half of that pair. Never committed — the directory is gitignored.
PRIVATE_KEY_PATH: Final[Path] = (
    _REPO_ROOT / "deploy" / "compose" / "authenticator-dev-keys" / "testclient.key.pem"
)

#: Point a runner that cannot see the repo at the key and the endpoint.
KEY_PATH_ENV: Final[str] = "INSIGHT_STAND_SERVICE_KEY"
TOKEN_URL_ENV: Final[str] = "INSIGHT_STAND_TOKEN_URL"
AUDIENCE_ENV: Final[str] = "INSIGHT_STAND_TOKEN_AUDIENCE"
IDENTITY_URL_ENV: Final[str] = "INSIGHT_STAND_IDENTITY_URL"

#: identity-resolution's own listener, where a SERVICE reaches `/internal/*`.
IDENTITY_PORT_KEY: Final[str] = "IDENTITY_RESOLUTION_PORT"
DEFAULT_IDENTITY_PORT: Final[str] = "8086"

#: Where the authenticator's second listener is published, and what it compares
#: `aud` against. Both mirror docker-compose.yml's defaults.
TOKEN_PORT_KEY: Final[str] = "AUTHENTICATOR_TOKEN_PORT"
DEFAULT_TOKEN_PORT: Final[str] = "8093"
AUDIENCE_KEY: Final[str] = "AUTHENTICATOR_TOKEN_AUDIENCE"
DEFAULT_AUDIENCE: Final[str] = "http://authenticator:8093/internal/token"

#: The authenticator caps an assertion's `exp - iat` at 60s and replay-guards
#: its `jti`. Well inside that, and a fresh assertion per exchange.
ASSERTION_LIFETIME_S: Final[int] = 30

#: Re-exchange before the issued token expires (its TTL is 300s by default), for
#: the same reason `LoginSession` re-logs-in early: a long suite must not have a
#: credential die mid-run.
DEFAULT_MAX_TOKEN_AGE_S: Final[float] = 240.0


def _env_file() -> Path | None:
    for name in CANDIDATE_ENV_FILES:
        candidate = _REPO_ROOT / name
        if candidate.is_file():
            return candidate
    return None


def default_token_url(environ: Mapping[str, str] | None = None) -> str:
    """Where to POST the assertion, for THIS runner.

    `$INSIGHT_STAND_TOKEN_URL` first — set by `dev-compose.sh` when the suite
    runs inside the ui-tests image, where the listener is reachable only by its
    in-network name because that runner shares the gateway's network namespace
    and `localhost` is the gateway's.

    Otherwise the published host port, read from the stand's own env file so a
    stand on a non-default port is found where it actually listens.
    """
    import os

    env = os.environ if environ is None else environ
    explicit = (env.get(TOKEN_URL_ENV) or "").strip()
    if explicit:
        return explicit.rstrip("/")

    port = DEFAULT_TOKEN_PORT
    found = _env_file()
    if found is not None:
        port = parse_env_file(found).get(TOKEN_PORT_KEY, "").strip() or DEFAULT_TOKEN_PORT
    return f"http://{PUBLISHED_HOST}:{port}"


def default_audience(environ: Mapping[str, str] | None = None) -> str:
    """The `aud` the authenticator compares against — a CONFIGURED value.

    Deliberately not derived from `default_token_url`: the authenticator checks
    one fixed audience whoever is calling, so a host-side caller must still send
    the in-network form. Reading it from the stand's env file is what keeps the
    two in step when someone overrides it.
    """
    import os

    env = os.environ if environ is None else environ
    explicit = (env.get(AUDIENCE_ENV) or "").strip()
    if explicit:
        return explicit

    found = _env_file()
    if found is not None:
        configured = parse_env_file(found).get(AUDIENCE_KEY, "").strip()
        if configured:
            return configured
    return DEFAULT_AUDIENCE


def default_identity_url(environ: Mapping[str, str] | None = None) -> str:
    """identity-resolution's own base url — NOT the gateway's.

    A service principal cannot reach `/internal/*` through the edge, and that is
    the product behaving correctly rather than a gap to work around. The gateway
    is a browser BFF: it delegates authz to the authenticator, which looks for a
    session cookie and answers `401 no_session` to a request carrying a bearer
    token instead. Real callers — the authenticator resolving a person during
    login — address the service in-network, so this suite does too.

    The negative half stays at the edge on purpose: `test_internal_lookup_
    refuses_a_person` goes through `/api/identity/...` because a human's refusal
    is exactly what the gateway path should produce. Same route, two addresses,
    each proving the thing it is able to prove.

    Resolved like `default_token_url`: an explicit override first, else the
    published host port from the stand's own env file.
    """
    import os

    env = os.environ if environ is None else environ
    explicit = (env.get(IDENTITY_URL_ENV) or "").strip()
    if explicit:
        return explicit.rstrip("/")

    port = DEFAULT_IDENTITY_PORT
    found = _env_file()
    if found is not None:
        port = parse_env_file(found).get(IDENTITY_PORT_KEY, "").strip() or DEFAULT_IDENTITY_PORT
    return f"http://{PUBLISHED_HOST}:{port}"


def read_private_key(path: Path | None = None) -> str:
    """The `testclient` private key, or an error naming how to get one."""
    import os

    override = (os.environ.get(KEY_PATH_ENV) or "").strip()
    target = Path(override) if override else (path or PRIVATE_KEY_PATH)
    try:
        return target.read_text(encoding="utf-8")
    except OSError as exc:
        raise PersonaError(
            f"no service-principal key at {target}: {exc}. It is generated per checkout by "
            f"./dev-compose.sh test-stand up (never committed); set ${KEY_PATH_ENV} to point "
            "a runner that cannot see the repo at its own copy."
        ) from exc


@dataclass
class ServiceTokenSession:
    """A service principal's credential, exchanged at the deployed endpoint.

    Interchangeable with `LoginSession` wherever a session is attached — both
    answer `headers()` — so an `ApiClient` carries one without knowing which it
    holds.
    """

    tenant_id: str
    token_url: str
    audience: str
    private_key_pem: str = field(repr=False)
    service: str = SERVICE_NAME
    timeout_s: float = 30.0
    max_token_age_s: float = DEFAULT_MAX_TOKEN_AGE_S
    _token: str | None = field(default=None, init=False, repr=False)
    _acquired_at: float = field(default=0.0, init=False, repr=False)

    # -- what a request carries --------------------------------------------

    def headers(self) -> dict[str, str]:
        if self._token is None or self._is_stale():
            self.exchange()
        return {"Authorization": f"Bearer {self._token}"}

    def is_authenticated(self) -> bool:
        return self._token is not None and not self._is_stale()

    def _is_stale(self) -> bool:
        return (time.monotonic() - self._acquired_at) > self.max_token_age_s

    # -- the exchange ------------------------------------------------------

    def assertion(self) -> str:
        """One short-lived, single-use RFC 7523 assertion.

        `iss == sub == service` is what the authenticator requires (it reads
        `iss` to select the registered key set, then rejects a `sub` that
        disagrees), and a fresh `jti` every time because a used one is
        replay-guarded for its whole lifetime.
        """
        import jwt

        now = int(time.time())
        return jwt.encode(
            {
                "iss": self.service,
                "sub": self.service,
                "aud": self.audience,
                "iat": now,
                "exp": now + ASSERTION_LIFETIME_S,
                "jti": str(uuid.uuid4()),
            },
            self.private_key_pem,
            algorithm="ES256",
        )

    def exchange(self) -> None:
        """Trade an assertion for a gateway JWT, or raise saying what refused.

        A 401 here is deliberately uninformative on the wire — the endpoint
        will not say why an assertion was rejected — so the message names the
        inputs a reader can actually check instead of pretending to explain.
        """
        import httpx

        endpoint = f"{self.token_url}/internal/token"
        try:
            response = httpx.post(
                endpoint,
                data={
                    "grant_type": "client_credentials",
                    "client_assertion_type": ASSERTION_TYPE,
                    "client_assertion": self.assertion(),
                    # Service tokens are tenant-scoped and the handler refuses a
                    # request naming none.
                    "tenant_id": self.tenant_id,
                },
                timeout=self.timeout_s,
            )
        except httpx.HTTPError as exc:
            raise PersonaError(
                f"cannot reach the service-token endpoint at {endpoint}: {exc}. On the host it is "
                f"the published {TOKEN_PORT_KEY}; inside the ui-tests image it is the in-network "
                f"name, which ${TOKEN_URL_ENV} carries."
            ) from exc

        if response.status_code != 200:
            raise PersonaError(
                f"{endpoint} refused the {self.service!r} assertion "
                f"(status {response.status_code}): {response.text[:300]}\n"
                f"  audience sent: {self.audience!r} (must equal the authenticator's configured "
                f"{AUDIENCE_KEY}, which is NOT the URL above)\n"
                f"  tenant_id sent: {self.tenant_id!r}"
            )

        body: Any = response.json()
        token = body.get("access_token") if isinstance(body, dict) else None
        if not isinstance(token, str) or not token:
            raise PersonaError(
                f"{endpoint} answered 200 with no access_token: {response.text[:300]}"
            )

        self._token = token
        self._acquired_at = time.monotonic()

    def forget(self) -> None:
        """Drop the token so the next `headers()` exchanges a new one."""
        self._token = None
        self._acquired_at = 0.0


def open_service_session(
    tenant_id: str,
    *,
    token_url: str | None = None,
    audience: str | None = None,
    key_path: Path | None = None,
) -> ServiceTokenSession:
    """A service principal scoped to `tenant_id`, ready to carry."""
    return ServiceTokenSession(
        tenant_id=tenant_id,
        token_url=token_url or default_token_url(),
        audience=audience or default_audience(),
        private_key_pem=read_private_key(key_path),
    )


__all__: Sequence[str] = (
    "ASSERTION_TYPE",
    "AUDIENCE_ENV",
    "IDENTITY_URL_ENV",
    "KEY_PATH_ENV",
    "PRIVATE_KEY_PATH",
    "SERVICE_NAME",
    "TOKEN_URL_ENV",
    "ServiceTokenSession",
    "default_audience",
    "default_identity_url",
    "default_token_url",
    "open_service_session",
    "read_private_key",
)
