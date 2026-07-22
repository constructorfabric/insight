"""Dev/e2e gateway-JWT auth for the full-auth ingestion rig.

The bronze->API rig runs analytics auth-ENABLED (NGINX_BFF R1): each request
carries a signed ES256 gateway JWT whose `tenant_id` claim is the sole tenant
authority (the removed `X-Insight-Tenant-Id` dev header is gone). Analytics'
`cf-gears-oidc-authn-plugin` verifies the JWT and resolves the JWKS via OIDC
discovery over HTTPS ONLY, so this module:

  * generates an ephemeral EC P-256 signing key (kid = RFC 7638 thumbprint),
  * serves `/.well-known/openid-configuration` + `/.well-known/jwks.json` from a
    self-signed TLS front on loopback (SAN 127.0.0.1), trusted by the plugin via
    `http_client.custom_ca_certificate_paths`,
  * mints per-tenant gateway JWTs for the harness to send as `Authorization`.

Everything is loopback + ephemeral; nothing here is a production credential.
"""

from __future__ import annotations

import base64
import datetime as _dt
import hashlib
import http.server
import ipaddress
import ssl
import tempfile
import threading
import time
import uuid
from pathlib import Path

import jwt as _jwt
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID

AUDIENCE = "internal-services"


def _b64url(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


class GatewayAuth:
    """Owns the signing key, the TLS discovery front, and per-tenant minting."""

    def __init__(self) -> None:
        self._key = ec.generate_private_key(ec.SECP256R1())
        self._key_pem = self._key.private_bytes(
            serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()
        ).decode()

        nums = self._key.public_key().public_numbers()
        x_b64 = _b64url(nums.x.to_bytes(32, "big"))
        y_b64 = _b64url(nums.y.to_bytes(32, "big"))
        # RFC 7638 EC thumbprint — must match the authenticator's kid scheme so
        # the plugin finds this key in the served JWKS.
        canonical = f'{{"crv":"P-256","kty":"EC","x":"{x_b64}","y":"{y_b64}"}}'
        self._kid = _b64url(hashlib.sha256(canonical.encode()).digest())
        self._jwk = {
            "kty": "EC",
            "crv": "P-256",
            "x": x_b64,
            "y": y_b64,
            "use": "sig",
            "alg": "ES256",
            "kid": self._kid,
        }

        self._certs_dir = Path(tempfile.mkdtemp(prefix="e2e-gwauth-"))
        self._ca_path = self._certs_dir / "ca.pem"
        self._start_tls_front()

    # -- TLS discovery front ---------------------------------------------------

    def _start_tls_front(self) -> None:
        cert_pem, key_pem = self._self_signed_cert()
        cert_path = self._certs_dir / "server.pem"
        key_path = self._certs_dir / "server.key"
        cert_path.write_bytes(cert_pem)
        key_path.write_bytes(key_pem)
        # The plugin trusts the self-signed leaf directly as a root.
        self._ca_path.write_bytes(cert_pem)

        # Bind to an ephemeral port; discover it after bind.
        docs = self._docs  # bound below once we know the port
        auth = self

        class _Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):  # noqa: N802
                body = docs().get(self.path)
                if body is None:
                    self.send_response(404)
                    self.end_headers()
                    return
                encoded = body.encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)

            def log_message(self, *_a):  # silence access logging
                pass

        self._server = http.server.HTTPServer(("127.0.0.1", 0), _Handler)
        self._port = self._server.server_address[1]
        self.issuer = f"https://127.0.0.1:{self._port}"

        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(certfile=str(cert_path), keyfile=str(key_path))
        self._server.socket = ctx.wrap_socket(self._server.socket, server_side=True)

        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()
        # Bind the doc bodies now that the issuer/port are known.
        _ = auth.issuer

    def _self_signed_cert(self) -> tuple[bytes, bytes]:
        key = ec.generate_private_key(ec.SECP256R1())
        subject = issuer = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "e2e-gwauth")])
        now = _dt.datetime(2020, 1, 1, tzinfo=_dt.UTC)
        cert = (
            x509.CertificateBuilder()
            .subject_name(subject)
            .issuer_name(issuer)
            .public_key(key.public_key())
            .serial_number(x509.random_serial_number())
            .not_valid_before(now)
            .not_valid_after(now + _dt.timedelta(days=3650))
            .add_extension(
                x509.SubjectAlternativeName([x509.IPAddress(ipaddress.ip_address("127.0.0.1"))]), critical=False
            )
            .sign(key, hashes.SHA256())
        )
        return (
            cert.public_bytes(serialization.Encoding.PEM),
            key.private_bytes(
                serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()
            ),
        )

    def _docs(self) -> dict[str, str]:
        import json

        return {
            "/.well-known/openid-configuration": json.dumps(
                {
                    "issuer": self.issuer,
                    "jwks_uri": f"{self.issuer}/.well-known/jwks.json",
                    "id_token_signing_alg_values_supported": ["ES256"],
                    "response_types_supported": ["code"],
                    "subject_types_supported": ["public"],
                }
            ),
            "/.well-known/jwks.json": json.dumps({"keys": [self._jwk]}),
        }

    # -- accessors -------------------------------------------------------------

    @property
    def ca_path(self) -> str:
        return str(self._ca_path)

    def mint(self, tenant_id: str, *, sub: str | None = None, roles: str = "analyst") -> str:
        """Sign an ES256 gateway JWT scoped to `tenant_id`."""
        now = int(time.time())
        claims = {
            "sub": sub or str(uuid.uuid4()),
            "tenant_id": tenant_id,
            "roles": roles,  # space-delimited scope string
            "sub_type": "user",
            "sid": str(uuid.uuid4()),
            "iss": self.issuer,
            "aud": AUDIENCE,
            "iat": now,
            "exp": now + 3600,
            "jti": str(uuid.uuid4()),
        }
        return _jwt.encode(claims, self._key_pem, algorithm="ES256", headers={"kid": self._kid})

    def auth_header(self, tenant_id: str) -> dict[str, str]:
        return {"Authorization": f"Bearer {self.mint(tenant_id)}"}

    def stop(self) -> None:
        try:
            self._server.shutdown()
            self._server.server_close()
        except Exception:  # noqa: BLE001 — best-effort teardown
            pass
