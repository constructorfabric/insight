"""Session orchestrator for the downstream-verification e2e.

Owns the compose stack lifecycle (fakeidp + authenticator + gateway + the REAL
analytics and identity services) and exposes a small HTTP client plus a
service-token minter. Tests live in test_downstream.py.

pytest runs on the host; the OIDC redirect chain uses in-network hostnames, so
the client rewrites them to the published localhost ports.
"""

from __future__ import annotations

import os
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

import pytest

HERE = Path(__file__).parent

# Exchange-cache window (seconds). Drives the authenticator's
# authz_cache_max_age_seconds via the compose `${AUTHZ_CACHE_MAX_AGE:-3}`
# default, so it must be exported into the compose subprocess environment
# before the stack starts (otherwise the harness never comes up).
AUTHZ_CACHE_MAX_AGE = int(os.environ.setdefault("AUTHZ_CACHE_MAX_AGE", "3"))

COMPOSE = ["docker", "compose", "-f", str(HERE / "docker-compose.e2e.yml")]
SERVICES = [
    "redis",
    "mariadb",
    "fakeidp",
    "identity-stub",
    "authenticator",
    "authn-tls",
    "analytics",
    "identity",
    "echo",
    "gateway",
]

GW = "http://localhost:18080"
FAKEIDP = "http://localhost:18084"
AUTHENTICATOR = "http://localhost:18083"
AUTH_TOKEN = "http://localhost:18093"  # authenticator token listener
ANALYTICS_DIRECT = "http://localhost:18081"  # bypasses the gateway (R1 proof)
IDENTITY_DIRECT = "http://localhost:18082"  # bypasses the gateway (R1 proof)

# The service-token assertion audience — must match the authenticator's baked
# service_tokens.audience (config/insight.yaml).
SERVICE_TOKEN_AUDIENCE = "http://localhost:8093/internal/token"

REWRITES = {"http://gateway:8080": GW, "http://fakeidp:8084": FAKEIDP}


def _compose(*args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run([*COMPOSE, *args], check=check, capture_output=True, text=True)


class Client:
    """Minimal HTTP client: no auto-redirects, case-insensitive headers, an OIDC
    login helper, and a form POST for the service-token exchange."""

    def request(self, url, headers=None, method="GET", data=None):
        body = None
        hdrs = dict(headers or {})
        if data is not None:
            body = urllib.parse.urlencode(data).encode()
            hdrs["Content-Type"] = "application/x-www-form-urlencoded"
        req = urllib.request.Request(url, headers=hdrs, method=method, data=body)

        class _NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, *a, **k):
                return None

        opener = urllib.request.build_opener(_NoRedirect)
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
        """Drive the OIDC code flow through the gateway; return the __Host-sid value.

        `user` optionally picks a fakeidp test user (by email) — the fake
        `/authorize` honours a `user=` query param; omit for the default dev user.
        """
        _, h, _ = self.request(f"{GW}/auth/login?return_to=/")
        authorize = self._rewrite(h["location"])  # fakeidp /authorize
        if user is not None:
            sep = "&" if "?" in authorize else "?"
            authorize = f"{authorize}{sep}user={urllib.parse.quote(user)}"
        _, h, _ = self.request(authorize)
        status, h, _ = self.request(self._rewrite(h["location"]))  # gateway /auth/callback
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
    # Trust the self-signed leaf directly as a root (it validates itself).
    (certs / "ca.pem").write_bytes((certs / "server.pem").read_bytes())
    for f in ("server.key", "server.pem", "ca.pem"):
        (certs / f).chmod(0o644)


@pytest.fixture(scope="session", autouse=True)
def stack():
    keys = HERE / "keys"
    keys.mkdir(exist_ok=True)
    # ES256 gateway signing key (EC P-256): the downstream verifiers — the
    # oidc-authn-plugin (analytics) and .NET JwtBearer (identity) — validate the
    # ES256 gateway JWT the authenticator signs.
    _genpkey_ec(keys / "current.pem")
    (keys / "current.pem").chmod(0o644)
    # TLS discovery front certs (authn-tls) into ./certs.
    certs = HERE / "certs"
    certs.mkdir(exist_ok=True)
    _gen_tls_certs(certs)
    # Service-token client keypair: the baked `testclient` registry entry
    # references testclient.pub.pem (resolved against public_key_dir=/keys). It
    # stays EC — the RFC 7523 client assertion is ES256.
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
    try:
        _compose("up", "-d", "--build", *SERVICES)
        _wait_http(f"{GW}/healthz", want={200})
        _wait_http(f"{GW}/auth/login", want={302})
        # Both downstream services up: /health is public on each host, and a
        # no-cookie /api/* through the gateway must already be 401 (auth wired).
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
    """Sign a gateway JWT directly with the e2e gateway key (keys/current.pem),
    using ES256 + the authenticator's RFC 7638 kid so the downstream plugin
    finds the key in the JWKS. Lets a test forge an otherwise-valid token that
    omits a required claim (e.g. `tenant_id`) to prove the plugin rejects it."""
    jwt = pytest.importorskip("jwt")  # PyJWT
    from cryptography.hazmat.primitives.serialization import load_pem_private_key

    key_pem = (HERE / "keys" / "current.pem").read_bytes()
    priv = load_pem_private_key(key_pem, password=None)
    kid = _gateway_kid(priv.public_key())
    return jwt.encode(claims, key_pem.decode(), algorithm="ES256", headers={"kid": kid})


def mint_service_token(tenant: str) -> str:
    """Run the step-06 RFC 7523 flow: sign a client assertion with the testclient
    key, exchange it at the authenticator's token endpoint for a gateway service
    token (sub = a per-service UUID, sub_type=service, roles include `service`).
    Requires PyJWT."""
    jwt = pytest.importorskip("jwt")  # PyJWT; skip scenario 4 if unavailable
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
