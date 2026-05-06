#!/usr/bin/env python3
"""Mint a short-lived HS256 JWT for Airbyte API access.

Reads the HMAC signing secret from stdin (raw bytes / decoded base64),
prints `<header>.<payload>.<signature>` to stdout. Mirrors the JWT shape
that airbyte-server expects: iss=airbyte-server, sub=instance-admin
workspace UUID, iat=now, exp=now+ttl.

CLI:
  echo -n "$JWT_SECRET" | mint_airbyte_jwt.py <ttl_seconds>

Stdout: JWT string.
Exit:   0 on success; non-zero on bad args.
"""
import base64
import hashlib
import hmac
import json
import sys
import time

INSTANCE_ADMIN_SUB = "00000000-0000-0000-0000-000000000000"
ISSUER = "airbyte-server"


def _b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def main() -> int:
    if len(sys.argv) != 2:
        sys.stderr.write("mint_airbyte_jwt: expected 1 arg (ttl_seconds)\n")
        return 2
    try:
        ttl = int(sys.argv[1])
    except ValueError:
        sys.stderr.write("mint_airbyte_jwt: ttl must be integer seconds\n")
        return 2
    if ttl < 30 or ttl > 3600:
        sys.stderr.write("mint_airbyte_jwt: ttl must be in [30, 3600]\n")
        return 2
    secret = sys.stdin.buffer.read()
    if not secret:
        sys.stderr.write("mint_airbyte_jwt: empty signing secret on stdin\n")
        return 2
    header = _b64url(json.dumps({"alg": "HS256", "typ": "JWT"}, separators=(",", ":")).encode())
    now = int(time.time())
    payload = _b64url(
        json.dumps(
            {"iss": ISSUER, "sub": INSTANCE_ADMIN_SUB, "iat": now, "exp": now + ttl},
            separators=(",", ":"),
        ).encode()
    )
    sig = _b64url(
        hmac.new(secret, f"{header}.{payload}".encode(), hashlib.sha256).digest()
    )
    sys.stdout.write(f"{header}.{payload}.{sig}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
