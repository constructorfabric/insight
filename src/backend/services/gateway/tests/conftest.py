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

REWRITES = {"http://gateway:8080": GW, "http://keycloak:8085": KEYCLOAK}

KC_REALM = "insight"
KC_DISCOVERY = f"{KEYCLOAK}/realms/{KC_REALM}/.well-known/openid-configuration"
E2E_USER = "dev@company.nonpresent"
E2E_PASSWORD = "insight-dev"
TENANT_ID = "00000000-df51-5b42-9538-d2b56b7ee953"

_LOGIN_FORM = re.compile(r'<form[^>]+action="([^"]*login-actions/authenticate[^"]*)"', re.IGNORECASE)


class _SendSecureOverHttp(http.cookiejar.DefaultCookiePolicy):
    """Keycloak marks its auth-session cookies Secure even over plain http, and
    the stdlib jar then refuses to send them back over the rig's published http
    port — the credential POST would arrive session-less and 400. A browser at
    http://localhost has no such problem (secure context)."""

    def return_ok_secure(self, cookie, request):
        return True


AUTHZ_CACHE_MAX_AGE = int(os.environ.setdefault("AUTHZ_CACHE_MAX_AGE", "10"))


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

        status, h, _ = self.request(self._rewrite(h["location"]))
        assert status == 302, f"callback expected 302, got {status}"
        for part in h.get("set-cookie", "").split(";"):
            part = part.strip()
            if part.startswith("__Host-sid="):
                return part[len("__Host-sid=") :]
        raise AssertionError(f"no __Host-sid in Set-Cookie: {h.get('set-cookie')!r}")

    def csrf_token(self, sid):
        status, _, body = self.request(f"{GW}/auth/csrf", headers={"Cookie": f"__Host-sid={sid}"})
        assert status == 200, f"/auth/csrf got {status}"
        return json.loads(body)["csrf_token"]


def _wait_http(url, want, timeout_s=90):
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

    _genpkey_ec("current.pem")
    (keys / "current.pem").chmod(0o644)
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
        # discovery document answers only once its import committed.
        _wait_http(KC_DISCOVERY, want={200}, timeout_s=240)
        _wait_http(f"{GW}/healthz", want={200})
        _wait_http(f"{GW}/auth/login", want={302})
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
    return GatewayClient().login()


def _wait_status(url, cookie, expected, timeout_s=30):
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        status, _, _ = GatewayClient().request(url, headers={"Cookie": cookie})
        if status == expected:
            return
        time.sleep(0.2)
    raise TimeoutError(f"{url} never returned {expected}")


@pytest.fixture
def authenticator_down(session_sid):
    status, _, _ = GatewayClient().request(f"{GW}/api/analytics/warm", headers={"Cookie": f"__Host-sid={session_sid}"})
    assert status == 200, f"warm got {status}"
    _compose("kill", "authenticator")
    _wait_status(f"{GW}/api/analytics/x", "__Host-sid=cold-poll", 503)
    yield
    _compose("start", "authenticator")
    _wait_http(f"{GW}/auth/login", want={302})


@pytest.fixture
def echo_down():
    _compose("kill", "echo")
    yield
    _compose("start", "echo")
