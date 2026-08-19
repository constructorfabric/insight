"""Session orchestrator for the gateway e2e (NGINX_BFF step-05 scenarios).

Owns the compose stack lifecycle (real authenticator + Keycloak behind the
OpenResty gateway, with stub identity + echo upstream) and exposes a small HTTP
client plus fixtures for the fail-closed scenarios (authenticator / upstream
down). Tests live in test_gateway.py.

The IdP is a real Keycloak importing the generated roster realm
(`insight-seed-realm`, from src/ingestion/tools/seed — generated here into
./keycloak-import). pytest runs on the host; the OIDC redirect chain uses
in-network hostnames, so the client rewrites them to the published localhost
ports (see GatewayClient).
"""

from __future__ import annotations

import http.cookiejar
import json
import os
import re
import shutil
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

import pytest

HERE = Path(__file__).parent
REPO_ROOT = HERE.parents[4]
COMPOSE = ["docker", "compose", "-f", str(HERE / "docker-compose.e2e.yml")]
CORE_SERVICES = ["redis", "identity-stub", "keycloak", "authenticator", "echo", "gateway"]

GW = "http://localhost:18080"
KEYCLOAK = "http://localhost:18084"
AUTHENTICATOR = "http://localhost:18083"

# In-network hostnames the authenticator and Keycloak emit in redirects and
# form actions -> published ports.
REWRITES = {"http://gateway:8080": GW, "http://keycloak:8085": KEYCLOAK}

KC_REALM = "insight"
KC_DISCOVERY = f"{KEYCLOAK}/realms/{KC_REALM}/.well-known/openid-configuration"
# The realm's dev-lead persona; the generator bakes one dev password for all users.
E2E_USER = "dev@company.nonpresent"
E2E_PASSWORD = "insight-dev"
# The generator requires a tenant; the identity-stub resolves any external id,
# so the value only has to be named.
TENANT_ID = "00000000-df51-5b42-9538-d2b56b7ee953"

# Anchored on login-actions/authenticate, not "the first <form>".
_LOGIN_FORM = re.compile(r'<form[^>]+action="([^"]*login-actions/authenticate[^"]*)"', re.IGNORECASE)


class _SendSecureOverHttp(http.cookiejar.DefaultCookiePolicy):
    """Keycloak marks its auth-session cookies Secure even over plain http, and
    the stdlib jar then refuses to send them back over the rig's published http
    port — the credential POST would arrive session-less and 400. A browser at
    http://localhost has no such problem (secure context)."""

    def return_ok_secure(self, cookie, request):
        return True


# Must match the authenticator's authz_cache_max_age_seconds in the compose file.
AUTHZ_CACHE_MAX_AGE = 3


def _compose(*args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run([*COMPOSE, *args], check=check, capture_output=True, text=True)


def _compose_up_args() -> list[str]:
    mode = os.environ.get("GATEWAY_E2E_PREBUILT")
    if mode == "true":
        return ["up", "-d", "--no-build", *CORE_SERVICES]
    if mode in {None, "false"}:
        return ["up", "-d", "--build", *CORE_SERVICES]
    raise ValueError("GATEWAY_E2E_PREBUILT must be 'true' or 'false'")


class GatewayClient:
    """Minimal HTTP client: no auto-redirects, case-insensitive headers, and an
    OIDC login helper that drives Keycloak's HTML login form, rewriting
    in-network redirect hosts to localhost."""

    def request(self, url, headers=None, method="GET", data=None, jar=None):
        body = None
        hdrs = dict(headers or {})
        if data is not None:
            body = urllib.parse.urlencode(data).encode()
            hdrs["Content-Type"] = "application/x-www-form-urlencoded"
        req = urllib.request.Request(url, headers=hdrs, method=method, data=body)

        class _NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, *a, **k):
                return None

        handlers: list = [_NoRedirect]
        if jar is not None:
            handlers.append(urllib.request.HTTPCookieProcessor(jar))
        opener = urllib.request.build_opener(*handlers)
        try:
            resp = opener.open(req, timeout=15)
            return resp.status, self._lower(resp.headers), resp.read()
        except urllib.error.HTTPError as e:
            return e.code, self._lower(e.headers), e.read()

    @staticmethod
    def _lower(headers):
        return {k.lower(): v for k, v in headers.items()}

    @staticmethod
    def _rewrite(url):
        for internal, external in REWRITES.items():
            url = url.replace(internal, external)
        return url

    def login(self):
        """Drive the OIDC code flow through the gateway; return the __Host-sid value.

        Keycloak serves a real HTML login form, so the middle of the chain is:
        GET the authorize URL (collecting the IdP's auth-session cookies), parse
        the form action, POST the credentials, then deliver the code redirect to
        the gateway callback.
        """
        _, h, _ = self.request(f"{GW}/auth/login?return_to=/")

        jar = http.cookiejar.CookieJar(policy=_SendSecureOverHttp())
        status, _, body = self.request(self._rewrite(h["location"]), jar=jar)
        assert status == 200, f"authorize expected the login form, got {status}"
        match = _LOGIN_FORM.search(body.decode())
        assert match, "no Keycloak login form in the authorize response"
        action = self._rewrite(match.group(1).replace("&amp;", "&"))

        status, h, _ = self.request(
            action, method="POST", data={"username": E2E_USER, "password": E2E_PASSWORD}, jar=jar
        )
        assert status == 302, f"credential POST expected 302, got {status}"

        status, h, _ = self.request(self._rewrite(h["location"]))  # gateway /auth/callback
        assert status == 302, f"callback expected 302, got {status}"
        for part in h.get("set-cookie", "").split(";"):
            part = part.strip()
            if part.startswith("__Host-sid="):
                return part[len("__Host-sid=") :]
        raise AssertionError(f"no __Host-sid in Set-Cookie: {h.get('set-cookie')!r}")

    def csrf_token(self, sid):
        """Fetch the per-session CSRF token; required on state-changing /auth/*."""
        status, _, body = self.request(f"{GW}/auth/csrf", headers={"Cookie": f"__Host-sid={sid}"})
        assert status == 200, f"/auth/csrf got {status}"
        return json.loads(body)["csrf_token"]


def _wait_http(url, want, timeout_s=90):
    # Poll until the endpoint returns one of `want`. A transient gateway 502
    # (e.g. the authenticator still booting after a restart) is NOT ready, so we
    # wait for the real status (302 for /auth/login, 200 for /healthz) rather
    # than "any response". A connection error (OSError) is also a retry.
    deadline = time.monotonic() + timeout_s
    last = None
    while time.monotonic() < deadline:
        try:
            last, _, _ = GatewayClient().request(url)
            if last in want:
                return
        except OSError:
            pass
        time.sleep(1)
    raise TimeoutError(f"not ready: {url} (last={last})")


def _generate_realm(import_dir: Path) -> None:
    """Generate the Keycloak import realm with `insight-seed-realm`.

    The redirect is passed explicitly: --authenticator-redirect REPLACES the
    defaults, which would deregister the gateway callback."""
    seed = REPO_ROOT / "src" / "ingestion" / "tools" / "seed"
    import_dir.mkdir(exist_ok=True)
    subprocess.run(
        [
            "uv",
            "run",
            "--project",
            str(seed),
            "insight-seed-realm",
            "--dev-email",
            E2E_USER,
            "--authenticator-redirect",
            "http://gateway:8080/auth/callback",
            "--out",
            str(import_dir / "realm-insight.json"),
        ],
        check=True,
        capture_output=True,
        env={**os.environ, "TENANT_DEFAULT_ID": TENANT_ID},
    )


@pytest.fixture(scope="session", autouse=True)
def stack():
    """Build + start the compose stack for the whole session; tear down after."""
    keys = HERE / "keys"
    keys.mkdir(exist_ok=True)

    def _genpkey_ec(out: str) -> None:
        subprocess.run(
            [
                "openssl",
                "genpkey",
                "-algorithm",
                "EC",
                "-pkeyopt",
                "ec_paramgen_curve:P-256",
                "-pkeyopt",
                "ec_param_enc:named_curve",
                "-out",
                str(keys / out),
            ],
            check=True,
            capture_output=True,
        )

    # ES256 gateway signing key (§9.6): EC P-256 — the authenticator's p256
    # loader requires an EC key, and downstream verifiers validate ES256.
    _genpkey_ec("current.pem")
    (keys / "current.pem").chmod(0o644)
    # Service-token registry key: the baked config/insight.yaml carries a dev
    # `testclient` entry (public_key_paths: [testclient.pub.pem], resolved
    # against public_key_dir=/keys in the e2e compose), so the authenticator
    # needs the public half present to build the registry and boot. This e2e
    # does not exercise service tokens; it just satisfies that dev entry. The
    # service-token client key stays EC (ES256 RFC 7523 assertion).
    _genpkey_ec("testclient.key.pem")
    subprocess.run(
        [
            "openssl",
            "pkey",
            "-in",
            str(keys / "testclient.key.pem"),
            "-pubout",
            "-out",
            str(keys / "testclient.pub.pem"),
        ],
        check=True,
        capture_output=True,
    )
    (keys / "testclient.pub.pem").chmod(0o644)
    kc_import = HERE / "keycloak-import"
    _generate_realm(kc_import)
    try:
        _compose(*_compose_up_args())
        # Keycloak start + realm import runs tens of seconds; the realm
        # discovery document answers only once its import committed. The
        # authenticator discovers per-op, so it needs no restart after this.
        _wait_http(KC_DISCOVERY, want={200}, timeout_s=240)
        _wait_http(f"{GW}/healthz", want={200})
        _wait_http(f"{GW}/auth/login", want={302})  # 302 once the authenticator is reachable
        yield
    finally:
        _compose("logs", "--no-color", check=False)
        _compose("down", "-v", "--remove-orphans", check=False)
        for leftover in ("current.pem", "testclient.key.pem", "testclient.pub.pem"):
            (keys / leftover).unlink(missing_ok=True)
        keys.rmdir()
        shutil.rmtree(kc_import, ignore_errors=True)


@pytest.fixture
def client():
    return GatewayClient()


@pytest.fixture(scope="session")
def session_sid():
    """Log in once and warm the exchange cache; return the session cookie value.

    Created before any fixture that kills the authenticator (those depend on it),
    so the cache is populated while the authenticator is still up.
    """
    sid = GatewayClient().login()
    GatewayClient().request(f"{GW}/api/analytics/warm", headers={"Cookie": f"__Host-sid={sid}"})
    return sid


def _wait_status(url, cookie, expected, timeout_s=30):
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        status, _, _ = GatewayClient().request(url, headers={"Cookie": cookie})
        if status == expected:
            return
        time.sleep(1)
    raise TimeoutError(f"{url} never returned {expected}")


@pytest.fixture
def authenticator_down(session_sid):
    """Kill the authenticator (session already warmed), restore on teardown."""
    _compose("kill", "authenticator")
    # Wait until the fail-closed state is actually reached before yielding.
    _wait_status(f"{GW}/api/analytics/x", "__Host-sid=cold-poll", 503)
    yield
    _compose("start", "authenticator")
    _wait_http(f"{GW}/auth/login", want={302})


@pytest.fixture
def echo_down():
    """Kill the echo upstream, restore on teardown."""
    _compose("kill", "echo")
    yield
    _compose("start", "echo")
