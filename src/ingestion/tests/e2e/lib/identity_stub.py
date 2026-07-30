"""In-process Identity stub for the bronze-to-api e2e rig (#1691).

A minimal loopback HTTP backend the analytics `get_person` handler resolves
against (`POST {identity_url}/v1/profiles` with `{value_type:"email", value:<email>}`):
a canned profile for one seeded email (→ 200) and 404 for every other. Lets the
persons endpoint exercise its real 200/404 contract, which is otherwise a
no-backend 500.

Resolves purely by the request `value` and ignores headers on purpose. Analytics
forwards the caller's gateway JWT (Authorization) on this hop (NGINX_BFF G1), but
the stub does not verify it — the REAL Identity service would (R1); the stub is
what keeps this a test-only backend.
"""

from __future__ import annotations

import json
import logging
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any
from urllib.parse import urlparse

LOG = logging.getLogger("e2e.identity-stub")

# The one person the stub resolves, keyed by `email`. The dict is the identity
# `ProfileResponse` body analytics deserializes then maps into its own `Person`.
# Field names must match `infra::identity::ProfileResponse` (snake_case); unknown
# fields are ignored and null-valued optionals may be omitted.
SEEDED_EMAIL = "e2e.person@example.com"
SEEDED_PERSON: dict[str, Any] = {
    "email": SEEDED_EMAIL,
    "display_name": "E2E Person",
    "first_name": "E2E",
    "last_name": "Person",
    "department": "Engineering",
    "division": "Product",
    "job_title": "Staff Engineer",
    "status": "active",
    "supervisor_email": None,
    "supervisor_name": None,
    "subordinates": [],
}

# An email the stub never resolves — the 404 (not-found) probe.
UNKNOWN_EMAIL = "nobody@example.com"

_PROFILES_PATH = "/v1/profiles"


class _Handler(BaseHTTPRequestHandler):
    """Serves POST /v1/profiles; the seeded map lives on `self.server`."""

    def do_POST(self) -> None:  # noqa: N802 — BaseHTTPRequestHandler API
        path = urlparse(self.path).path
        if path != _PROFILES_PATH:
            self._send(404, {"error": "not found", "path": path})
            return
        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length) if length else b""
        try:
            email = json.loads(raw).get("value", "")
        except json.JSONDecodeError:
            self._send(400, {"error": "invalid json body"})
            return
        person = self.server.people.get(email)  # type: ignore[attr-defined]
        if person is None:
            self._send(404, {"error": "person not found", "value": email})
        else:
            self._send(200, person)

    def _send(self, status: int, body: dict[str, Any]) -> None:
        payload = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *args: Any) -> None:  # silence per-request stderr spam
        LOG.debug("identity-stub request: %s", self.path)


class IdentityStub:
    """A threaded loopback Identity stub.

    Start it BEFORE the analytics process spawns and pass `url` into the analytics
    config as `identity_url`, so the persons handler resolves against it.
    """

    def __init__(self, people: dict[str, dict[str, Any]] | None = None) -> None:
        self._people = dict(people) if people is not None else {SEEDED_EMAIL: SEEDED_PERSON}
        self._server: ThreadingHTTPServer | None = None
        self._thread: threading.Thread | None = None

    def start(self) -> None:
        # Port 0 → the OS assigns a free loopback port; read it back off the
        # bound socket (no find-free-port race).
        server = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
        server.people = self._people  # type: ignore[attr-defined]
        self._server = server
        self._thread = threading.Thread(target=server.serve_forever, name="identity-stub", daemon=True)
        self._thread.start()
        LOG.info("identity stub listening on %s", self.url)

    @property
    def url(self) -> str:
        if self._server is None:
            raise RuntimeError("identity stub not started")
        host, port = self._server.server_address[:2]
        return f"http://{host}:{port}"

    def stop(self) -> None:
        if self._server is not None:
            self._server.shutdown()
            self._server.server_close()
            self._server = None
        if self._thread is not None:
            self._thread.join(timeout=5)
            self._thread = None
