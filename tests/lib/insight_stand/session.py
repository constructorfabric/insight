"""A session on the stand: winning one, holding it, attaching it.

`LoginSession` is the one thing a request can carry to prove who it is, and it
is a session cookie obtained by actually logging in at the stand's IdP. That is
the only kind that proves anything about the deployed product. Minting a bearer
token is the in-process rig's path
(`src/ingestion/tests/e2e/lib/gateway_jwt.py`); doing it here would exercise
JWT verification — which that rig already covers — and skip the login entirely.

There IS now a second implementation, and so an interface: `ServiceTokenSession`
(`service_token.py`) is a service principal, obtained by exchanging an RFC 7523
assertion at the authenticator's own token endpoint. `api.StandSession` is the
protocol both answer. `/internal/*` is reachable only to a
service, so without a second implementation that deployed surface would go
untested.

There is still no anonymous implementation — an unauthenticated client is an
`ApiClient` with no session at all; `api.StandSession` says why.
"""

from __future__ import annotations

import re
import time
from collections.abc import Sequence
from dataclasses import dataclass, field
from typing import TYPE_CHECKING
from urllib.parse import urlsplit

from .errors import LoginNotCompletedError

if TYPE_CHECKING:  # pragma: no cover - typing only
    import httpx

# The authenticator's browser entry point. The gateway's own 401 body names it:
# "No valid session; authenticate at /auth/login."
LOGIN_PATH = "/auth/login"

# Where the IdP sends the authorization code back to. Served by the gateway,
# which fronts both the SPA and /auth/* — see `_callback_on_stand`.
CALLBACK_PATH = "/auth/callback"

# Session cookie minted by the authenticator after a successful OIDC callback.
# `__Host-` is not decoration: it pins the cookie to a secure origin with
# Path=/ and no Domain. http://localhost counts as a secure context, which is
# why the flow is driven at a localhost origin.
SESSION_COOKIE_NAME = "__Host-sid"

# The stand issues the session with Max-Age=600. Re-login well inside that so a
# long suite cannot have a session expire mid-run — a whole class of flake for
# the price of one extra login.
DEFAULT_MAX_SESSION_AGE_S = 300.0

# Keycloak's login form posts to .../login-actions/authenticate. Matching on
# that rather than "the first <form>" survives extra forms on the page
# (language pickers, social-provider buttons).
_LOGIN_FORM_RE = re.compile(
    r'<form[^>]+action="([^"]*login-actions/authenticate[^"]*)"', re.IGNORECASE
)
# Keycloak renders the failure reason in a kc-feedback / alert-error block.
_FORM_ERROR_RE = re.compile(
    r'<span[^>]*class="[^"]*kc-feedback-text[^"]*"[^>]*>(.*?)</span>', re.IGNORECASE | re.DOTALL
)


@dataclass
class LoginSession:
    """A session on the stand, won by logging in for real at its own IdP.

    Drives the deployed five-step chain end to end, with no shortcut at any
    step: `GET /auth/login` → the authenticator starts authorization-code+PKCE
    and 302s to Keycloak → Keycloak renders its real HTML username/password
    form → the form is submitted → Keycloak 302s back with a code → the
    authenticator exchanges it, resolves the person through identity, opens a
    Redis session and sets `__Host-sid`.

    Two details that are easy to get wrong:

    * **The cookie is `Secure`, so it must be attached by hand.** Python's
      cookie jar will store it but refuses to *send* it over plain http, so
      `headers()` reads the value out of the jar and sets `Cookie:` explicitly.
      A browser has no such problem because http://localhost is a secure
      context.
    * **The code is delivered at the stand's own origin.** The IdP redirects to
      the authenticator's configured `redirect_uri`; on compose that is the
      front container's origin, which proxies only `/api/` and would serve the
      SPA for `/auth/callback` instead of reaching the authenticator. The
      gateway fronts both, so the query is replayed against `base_url` — the
      same handler, reached at an origin that is actually wired up. The
      protocol is untouched: the `redirect_uri` in both the authorize request
      and the token exchange stays whatever the authenticator was configured
      with.
    """

    base_url: str
    email: str
    # repr=False: pytest prints locals and fixture reprs in a traceback, and a
    # live Keycloak password has no business in a CI log.
    password: str = field(repr=False)
    session_cookie_name: str = SESSION_COOKIE_NAME
    login_path: str = LOGIN_PATH
    callback_path: str = CALLBACK_PATH
    timeout_s: float = 30.0
    max_session_age_s: float = DEFAULT_MAX_SESSION_AGE_S
    _session_cookie: str | None = field(default=None, init=False, repr=False)
    _acquired_at: float = field(default=0.0, init=False, repr=False)

    # -- what a request carries --------------------------------------------

    def headers(self) -> dict[str, str]:
        if self._session_cookie is None or self._is_stale():
            self.login()
        return {"Cookie": f"{self.session_cookie_name}={self._session_cookie}"}

    def is_authenticated(self) -> bool:
        return self._session_cookie is not None and not self._is_stale()

    def _is_stale(self) -> bool:
        return (time.monotonic() - self._acquired_at) > self.max_session_age_s

    # -- the flow ----------------------------------------------------------

    def login(self) -> None:
        """Run the real OIDC chain until a session cookie exists.

        Raises `LoginNotCompletedError`, naming the step that stopped it, if
        any link in the chain does not behave as the deployed flow requires.
        """
        import httpx

        with httpx.Client(
            base_url=self.base_url, timeout=self.timeout_s, follow_redirects=False
        ) as client:
            authorize_url = self._start(client)
            form_action, page = self._fetch_login_form(client, authorize_url)
            redirected_to = self._submit_credentials(client, form_action, page)
            self._deliver_code(client, redirected_to)

            cookie = client.cookies.get(self.session_cookie_name)
            if not cookie:
                raise LoginNotCompletedError(
                    f"the callback did not set {self.session_cookie_name!r} for {self.email!r}",
                    stopped_at=redirected_to,
                )
            self._session_cookie = cookie
            self._acquired_at = time.monotonic()

    def _start(self, client: httpx.Client) -> str:
        response = client.get(self.login_path)
        location = response.headers.get("location")
        if response.status_code not in (301, 302, 303, 307, 308) or not location:
            raise LoginNotCompletedError(
                f"GET {self.login_path} did not redirect to the IdP "
                f"(status {response.status_code})",
                stopped_at=str(response.url),
            )
        return str(location)

    def _fetch_login_form(self, client: httpx.Client, authorize_url: str) -> tuple[str, str]:
        response = client.get(authorize_url)
        if response.status_code != 200:
            raise LoginNotCompletedError(
                f"the IdP authorize endpoint answered {response.status_code}, not a login form",
                stopped_at=authorize_url,
            )
        match = _LOGIN_FORM_RE.search(response.text)
        if not match:
            raise LoginNotCompletedError(
                "the IdP returned no recognisable login form to submit",
                stopped_at=authorize_url,
            )
        action = match.group(1).replace("&amp;", "&")
        # Refuse to post the password anywhere but back at the IdP. Without
        # this a relative or rewritten action would resolve against `base_url`
        # (this client's base) and send the persona's credentials to the
        # product under test.
        idp_origin = urlsplit(authorize_url)
        target = urlsplit(action)
        if (target.scheme, target.netloc) != (idp_origin.scheme, idp_origin.netloc):
            raise LoginNotCompletedError(
                f"the login form posts to {action!r}, which is not the IdP origin "
                f"{idp_origin.scheme}://{idp_origin.netloc} — refusing to send credentials there",
                stopped_at=authorize_url,
            )
        return action, response.text

    def _submit_credentials(self, client: httpx.Client, action: str, page: str) -> str:
        response = client.post(
            action,
            data={"username": self.email, "password": self.password},
            headers={"Content-Type": "application/x-www-form-urlencoded"},
        )
        location = response.headers.get("location")
        if response.status_code in (301, 302, 303, 307, 308) and location:
            return str(location)
        # A rejected credential comes back as another 200 login page carrying
        # the reason, not as an error status.
        reason = _FORM_ERROR_RE.search(response.text) or _FORM_ERROR_RE.search(page)
        detail = re.sub(r"\s+", " ", reason.group(1)).strip() if reason else "no reason given"
        raise LoginNotCompletedError(
            f"the IdP rejected the login form for {self.email!r} "
            f"(status {response.status_code}): {detail}",
            stopped_at=action,
        )

    def _deliver_code(self, client: httpx.Client, redirected_to: str) -> None:
        response = client.get(self._callback_on_stand(redirected_to))
        if response.status_code >= 400:
            raise LoginNotCompletedError(
                f"the authenticator rejected the authorization code "
                f"(status {response.status_code}): {response.text[:300]}",
                stopped_at=redirected_to,
            )

    def _callback_on_stand(self, redirected_to: str) -> str:
        """Re-address the IdP's redirect at this stand's own origin."""
        parts = urlsplit(redirected_to)
        if parts.path != self.callback_path:
            raise LoginNotCompletedError(
                f"the IdP redirected to {parts.path!r}, not {self.callback_path!r}",
                stopped_at=redirected_to,
            )
        query = f"?{parts.query}" if parts.query else ""
        return f"{self.base_url.rstrip('/')}{self.callback_path}{query}"

    def logout(self) -> None:
        """Forget the session so the next `headers()` logs in again."""
        self._session_cookie = None
        self._acquired_at = 0.0


__all__: Sequence[str] = (
    "CALLBACK_PATH",
    "DEFAULT_MAX_SESSION_AGE_S",
    "LOGIN_PATH",
    "SESSION_COOKIE_NAME",
    "LoginSession",
)
