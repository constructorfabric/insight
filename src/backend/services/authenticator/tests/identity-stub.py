#!/usr/bin/env python3
"""Minimal Identity stub for the authenticator e2e runner (dev/CI only).

Answers `GET /v1/persons/{email}` with a `ResolveProfileCommandModel` carrying a
deterministic `insight_source_id`, so the login loop can resolve a person
without standing up the real .NET Identity service + seeding. Any other path
404s. Bind address from argv[1] (default 127.0.0.1:8092).
"""
import hashlib
import json
import sys
import uuid
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import unquote


def person_id_for(email: str) -> str:
    # Deterministic UUID from the email (stable across calls within a run).
    digest = hashlib.sha256(f"identity-stub:{email}".encode()).digest()
    return str(uuid.UUID(bytes=digest[:16]))


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802
        if self.path.startswith("/v1/persons/"):
            email = unquote(self.path[len("/v1/persons/"):])
            body = json.dumps({
                "value_type": "email",
                "value": email,
                "insight_source_type": "person",
                "insight_source_id": person_id_for(email),
            }).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, *_args):  # silence access logging
        pass


if __name__ == "__main__":
    host, _, port = (sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1:8092").partition(":")
    HTTPServer((host, int(port)), Handler).serve_forever()
