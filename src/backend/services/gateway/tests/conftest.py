"""Session orchestrator for the gateway e2e (NGINX_BFF step-05 scenarios).

Owns the compose stack lifecycle (real authenticator + fakeidp behind the
OpenResty gateway, with stub identity + echo upstream) and exposes a small HTTP
client plus fixtures for the fail-closed scenarios (authenticator / upstream
down). Tests live in test_gateway.py.

pytest runs on the host; the OIDC redirect chain uses in-network hostnames, so
the client rewrites them to the published localhost ports (see GatewayClient).
"""

from __future__ import annotations

import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path

import pytest

HERE = Path(__file__).parent
COMPOSE = ["docker", "compose", "-f", str(HERE / "docker-compose.e2e.yml")]
CORE_SERVICES = ["redis", "identity-stub", "fakeidp", "authenticator", "echo", "gateway"]

GW = "http://localhost:18080"
FAKEIDP = "http://localhost:18084"
AUTHENTICATOR = "http://localhost:18083"

# In-network hostnames the authenticator emits in redirects -> published ports.
REWRITES = {"http://gateway:8080": GW, "http://fakeidp:8084": FAKEIDP}

# Must match the authenticator's authz_cache_max_age_seconds in the compose file.
AUTHZ_CACHE_MAX_AGE = 3


def _compose(*args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run([*COMPOSE, *args], check=check, capture_output=True, text=True)


class GatewayClient:
    """Minimal HTTP client: no auto-redirects, case-insensitive headers, and an
    OIDC login helper that rewrites in-network redirect hosts to localhost."""

    def request(self, url, headers=None, method="GET"):
        req = urllib.request.Request(url, headers=headers or {}, method=method)

        class _NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, *a, **k):
                return None

        opener = urllib.request.build_opener(_NoRedirect)
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
        """Drive the OIDC code flow through the gateway; return the __Host-sid value."""
        _, h, _ = self.request(f"{GW}/auth/login?return_to=/")
        _, h, _ = self.request(self._rewrite(h["location"]))  # fakeidp /authorize
        status, h, _ = self.request(self._rewrite(h["location"]))  # gateway /auth/callback
        assert status == 302, f"callback expected 302, got {status}"
        for part in h.get("set-cookie", "").split(";"):
            part = part.strip()
            if part.startswith("__Host-sid="):
                return part[len("__Host-sid=") :]
        raise AssertionError(f"no __Host-sid in Set-Cookie: {h.get('set-cookie')!r}")


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


@pytest.fixture(scope="session", autouse=True)
def stack():
    """Build + start the compose stack for the whole session; tear down after."""
    keys = HERE / "keys"
    keys.mkdir(exist_ok=True)

    def _genpkey(out: str) -> None:
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

    # ES256 gateway signing key.
    _genpkey("current.pem")
    (keys / "current.pem").chmod(0o644)
    # Service-token registry key: the baked config/insight.yaml carries a dev
    # `testclient` entry (public_key_paths: [testclient.pub.pem], resolved
    # against public_key_dir=/keys in the e2e compose), so the authenticator
    # needs the public half present to build the registry and boot. This e2e
    # does not exercise service tokens; it just satisfies that dev entry.
    _genpkey("testclient.key.pem")
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
    try:
        _compose("up", "-d", "--build", *CORE_SERVICES)
        _wait_http(f"{GW}/healthz", want={200})
        _wait_http(f"{GW}/auth/login", want={302})  # 302 once the authenticator is reachable
        yield
    finally:
        _compose("logs", "--no-color", check=False)
        _compose("down", "-v", "--remove-orphans", check=False)
        for leftover in ("current.pem", "testclient.key.pem", "testclient.pub.pem"):
            (keys / leftover).unlink(missing_ok=True)
        keys.rmdir()


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
    GatewayClient().request(f"{GW}/api/v1/analytics/warm", headers={"Cookie": f"__Host-sid={sid}"})
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
    _wait_status(f"{GW}/api/v1/analytics/x", "__Host-sid=cold-poll", 503)
    yield
    _compose("start", "authenticator")
    _wait_http(f"{GW}/auth/login", want={302})


@pytest.fixture
def echo_down():
    """Kill the echo upstream, restore on teardown."""
    _compose("kill", "echo")
    yield
    _compose("start", "echo")
