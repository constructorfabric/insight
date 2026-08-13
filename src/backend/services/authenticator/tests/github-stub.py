#!/usr/bin/env python3
"""Rig-local GitHub for the brokered-login e2e suite (dev/CI only).

Keycloak's built-in `github` identity provider accepts `baseUrl`/`apiUrl`
overrides (its GitHub-Enterprise seam); the realm overlay points both at this
stub, so the rig drives the REAL social importer — token exchange, profile,
primary-verified-email lookup — against a GitHub it controls:

- `GET  /login/oauth/authorize` — auto-approves and redirects straight back
  with a code; `e2e_user` (`known` | `unknown`, default `known`) picks which
  GitHub identity the code represents.
- `POST /login/oauth/access_token` — exchanges the code; the client id/secret
  must match the pair the realm registration carries.
- `GET  /user` — the profile, with a null `email`, forcing the provider down
  the /user/emails path.
- `GET  /user/emails` — a non-primary decoy first, then the primary verified
  email the sign-in must identify the user by.

`unknown`'s email carries the identity stub's `unknown-` refusal prefix, so
its brokered login completes at Keycloak and is refused downstream by the
authenticator — the unknown-person path with a fully working upstream.

Bind address from argv[1] (default 0.0.0.0:8098 — the Keycloak container
dials in via host.docker.internal, so loopback is not enough).
"""

import base64
import json
import secrets
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlencode, urlsplit

CLIENT_ID = "github-e2e-client"
CLIENT_SECRET = "github-e2e-secret"

USERS = {
    "known": {"id": 7100001, "login": "rig-broker-known", "name": "Broker Rig", "email": "broker-known@example.com"},
    "unknown": {
        "id": 7100002,
        "login": "rig-broker-unknown",
        "name": "Broker Rig",
        "email": "unknown-broker@example.com",
    },
}

codes: dict[str, str] = {}  # one-shot authorization code -> USERS key
tokens: dict[str, str] = {}  # access token -> USERS key


def _client_authenticated(form: dict, authorization: str) -> bool:
    # Keycloak may send the client pair as body params or as Basic auth.
    if (form.get("client_id") or [""])[0] == CLIENT_ID and (form.get("client_secret") or [""])[0] == CLIENT_SECRET:
        return True
    if authorization.startswith("Basic "):
        expected = base64.b64encode(f"{CLIENT_ID}:{CLIENT_SECRET}".encode()).decode()
        return authorization.removeprefix("Basic ") == expected
    return False


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        split = urlsplit(self.path)
        query = parse_qs(split.query)

        if split.path == "/login/oauth/authorize":
            self._authorize(query)
        elif split.path == "/user":
            self._json_for_bearer(lambda u: {"id": u["id"], "login": u["login"], "name": u["name"], "email": None})
        elif split.path == "/user/emails":
            self._json_for_bearer(
                lambda u: [
                    {"email": f"decoy-{u['login']}@example.com", "primary": False, "verified": True},
                    {"email": u["email"], "primary": True, "verified": True},
                ]
            )
        elif split.path == "/healthz":
            self._respond(200, b"ok", "text/plain")
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        if urlsplit(self.path).path != "/login/oauth/access_token":
            self.send_response(404)
            self.end_headers()
            return

        length = int(self.headers.get("Content-Length") or 0)
        form = parse_qs(self.rfile.read(length).decode())
        if not _client_authenticated(form, self.headers.get("Authorization") or ""):
            self._respond(401, b'{"error":"incorrect_client_credentials"}', "application/json")
            return
        user_key = codes.pop((form.get("code") or [""])[0], None)
        if user_key is None:
            self._respond(400, b'{"error":"bad_verification_code"}', "application/json")
            return

        token = secrets.token_hex(16)
        tokens[token] = user_key
        body = json.dumps({"access_token": token, "token_type": "bearer", "scope": "user:email"}).encode()
        self._respond(200, body, "application/json")

    def _authorize(self, query: dict) -> None:
        redirect_uri = (query.get("redirect_uri") or [""])[0]
        state = (query.get("state") or [""])[0]
        user_key = (query.get("e2e_user") or ["known"])[0]
        if (query.get("client_id") or [""])[0] != CLIENT_ID or user_key not in USERS or not redirect_uri:
            self.send_response(400)
            self.end_headers()
            return

        code = secrets.token_hex(16)
        codes[code] = user_key
        sep = "&" if "?" in redirect_uri else "?"
        self.send_response(302)
        self.send_header("Location", f"{redirect_uri}{sep}{urlencode({'code': code, 'state': state})}")
        self.end_headers()

    def _json_for_bearer(self, build) -> None:
        auth = self.headers.get("Authorization") or ""
        token = auth.removeprefix("Bearer ").removeprefix("token ")
        user_key = tokens.get(token)
        if user_key is None:
            self._respond(401, b'{"message":"Bad credentials"}', "application/json")
            return
        self._respond(200, json.dumps(build(USERS[user_key])).encode(), "application/json")

    def _respond(self, status: int, body: bytes, content_type: str) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):  # silence access logging
        pass


if __name__ == "__main__":
    host, _, port = (sys.argv[1] if len(sys.argv) > 1 else "0.0.0.0:8098").partition(":")
    ThreadingHTTPServer((host, int(port)), Handler).serve_forever()
