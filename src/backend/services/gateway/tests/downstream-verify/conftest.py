from __future__ import annotations

import http.cookiejar
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
REPO_ROOT = HERE.parents[5]

AUTHZ_CACHE_MAX_AGE = int(os.environ.setdefault("AUTHZ_CACHE_MAX_AGE", "3"))

COMPOSE = ["docker", "compose", "-f", str(HERE / "docker-compose.e2e.yml")]
SERVICES = [
    "redis",
    "mariadb",
    "keycloak",
    "identity-stub",
    "authenticator",
    "authn-tls",
    "analytics",
    "identity-resolution-migrate",
    "identity-resolution",
    "echo",
    "gateway",
]

GW = "http://localhost:18080"
KEYCLOAK = "http://localhost:18084"
AUTHENTICATOR = "http://localhost:18083"
AUTH_TOKEN = "http://localhost:18093"
ANALYTICS_DIRECT = "http://localhost:18081"
IDENTITY_DIRECT = "http://localhost:18082"

SERVICE_TOKEN_AUDIENCE = "http://localhost:8093/internal/token"

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


def _compose(*args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run([*COMPOSE, *args], check=check, capture_output=True, text=True)


class Client:

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
            resp = opener.open(req, timeout=20)
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

    def login(self, user=None):
        _, h, _ = self.request(f"{GW}/auth/login?return_to=/")

        jar = http.cookiejar.CookieJar(policy=_SendSecureOverHttp())
        status, _, body = self.request(self._rewrite(h["location"]), jar=jar)
        assert status == 200, f"authorize expected the login form, got {status}"
        match = _LOGIN_FORM.search(body.decode())
        assert match, "no Keycloak login form in the authorize response"
        action = self._rewrite(match.group(1).replace("&amp;", "&"))

        status, h, _ = self.request(
            action, method="POST", data={"username": user or E2E_USER, "password": E2E_PASSWORD}, jar=jar
        )
        assert status == 302, f"credential POST expected 302, got {status}"

        status, h, _ = self.request(self._rewrite(h["location"]))
        assert status == 302, f"callback expected 302, got {status}"
        for part in h.get("set-cookie", "").split(";"):
            part = part.strip()
            if part.startswith("__Host-sid="):
                return part[len("__Host-sid=") :]
        raise AssertionError(f"no __Host-sid in Set-Cookie: {h.get('set-cookie')!r}")


def _wait_http(url, want, timeout_s=120):
    deadline = time.monotonic() + timeout_s
    last = None
    while time.monotonic() < deadline:
        try:
            last, _, _ = Client().request(url)
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


def _genpkey_ec(path: Path) -> None:
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
            str(path),
        ],
        check=True,
        capture_output=True,
    )


def _gen_tls_certs(certs: Path) -> None:
    """Self-signed EC cert (SAN=authn-tls) for the TLS discovery front, plus a
    CA copy analytics trusts via custom_ca_certificate_paths. reqwest still does
    hostname verification against the SAN even for a custom root, so the SAN
    must be `authn-tls` (the in-network name the plugin connects to)."""
    _genpkey_ec(certs / "server.key")
    cnf = certs / "openssl.cnf"
    cnf.write_text(
        "[req]\n"
        "distinguished_name = dn\n"
        "x509_extensions = v3\n"
        "prompt = no\n"
        "[dn]\n"
        "CN = authn-tls\n"
        "[v3]\n"
        "subjectAltName = DNS:authn-tls\n"
    )
    subprocess.run(
        [
            "openssl",
            "req",
            "-x509",
            "-key",
            str(certs / "server.key"),
            "-out",
            str(certs / "server.pem"),
            "-days",
            "2",
            "-config",
            str(cnf),
        ],
        check=True,
        capture_output=True,
    )
    (certs / "ca.pem").write_bytes((certs / "server.pem").read_bytes())
    for f in ("server.key", "server.pem", "ca.pem"):
        (certs / f).chmod(0o644)


@pytest.fixture(scope="session", autouse=True)
def stack():
    keys = HERE / "keys"
    keys.mkdir(exist_ok=True)
    _genpkey_ec(keys / "current.pem")
    (keys / "current.pem").chmod(0o644)
    certs = HERE / "certs"
    certs.mkdir(exist_ok=True)
    _gen_tls_certs(certs)
    _genpkey_ec(keys / "testclient.key.pem")
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
        _compose("up", "-d", "--build", *SERVICES)
        # Keycloak start + realm import runs tens of seconds; the realm
        # discovery document answers only once its import committed.
        _wait_http(KC_DISCOVERY, want={200}, timeout_s=240)
        _wait_http(f"{GW}/healthz", want={200})
        _wait_http(f"{GW}/auth/login", want={302})
        _wait_http(f"{ANALYTICS_DIRECT}/health", want={200})
        _wait_http(f"{IDENTITY_DIRECT}/healthz", want={200})
        yield
    finally:
        _compose("logs", "--no-color", check=False)
        _compose("down", "-v", "--remove-orphans", check=False)
        for leftover in ("current.pem", "testclient.key.pem", "testclient.pub.pem"):
            (keys / leftover).unlink(missing_ok=True)
        keys.rmdir()
        for leftover in ("server.key", "server.pem", "ca.pem", "openssl.cnf"):
            (certs / leftover).unlink(missing_ok=True)
        certs.rmdir()
        shutil.rmtree(kc_import, ignore_errors=True)


@pytest.fixture
def client():
    return Client()


def _gateway_kid(pub) -> str:
    """RFC 7638 EC thumbprint (the authenticator's kid scheme): base64url-nopad
    SHA-256 of `{"crv":"P-256","kty":"EC","x":"..","y":".."}`."""
    import base64
    import hashlib

    nums = pub.public_numbers()

    def b64(b: bytes) -> str:
        return base64.urlsafe_b64encode(b).rstrip(b"=").decode()

    xb = b64(nums.x.to_bytes(32, "big"))
    yb = b64(nums.y.to_bytes(32, "big"))
    canonical = f'{{"crv":"P-256","kty":"EC","x":"{xb}","y":"{yb}"}}'
    return b64(hashlib.sha256(canonical.encode()).digest())


def mint_gateway_jwt(claims: dict) -> str:
    jwt = pytest.importorskip("jwt")
    from cryptography.hazmat.primitives.serialization import load_pem_private_key

    key_pem = (HERE / "keys" / "current.pem").read_bytes()
    priv = load_pem_private_key(key_pem, password=None)
    kid = _gateway_kid(priv.public_key())
    return jwt.encode(claims, key_pem.decode(), algorithm="ES256", headers={"kid": kid})


def mint_service_token(tenant: str) -> str:
    jwt = pytest.importorskip("jwt")
    key_pem = (HERE / "keys" / "testclient.key.pem").read_text()
    now = int(time.time())
    assertion = jwt.encode(
        {
            "iss": "testclient",
            "sub": "testclient",
            "aud": SERVICE_TOKEN_AUDIENCE,
            "jti": f"e2e-{now}-{time.monotonic_ns()}",
            "iat": now,
            "exp": now + 50,
        },
        key_pem,
        algorithm="ES256",
    )
    status, _, body = Client().request(
        f"{AUTH_TOKEN}/internal/token",
        method="POST",
        data={
            "grant_type": "client_credentials",
            "client_assertion_type": "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
            "client_assertion": assertion,
            "tenant_id": tenant,
        },
    )
    assert status == 200, f"token exchange failed: {status} {body!r}"
    import json

    return json.loads(body)["access_token"]
