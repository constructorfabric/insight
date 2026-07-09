"""In-process Identity stub for the bronze-to-api e2e rig (#1691).

A minimal loopback HTTP backend the analytics `get_person` handler resolves
against (`GET {identity_url}/v1/persons/{email}`): a canned `Person` for one
seeded email (→ 200) and 404 for every other. Lets the persons endpoint exercise
its real 200/404 contract, which is otherwise a no-backend 500.

Answers purely by email and ignores headers on purpose: the analytics client
sends no caller header (the api-gateway injects it in production), so the REAL
Identity service would 401 the header-less call → 500. The stub is what keeps
this a test-only change; a real backend would need an analytics code change.
"""

from __future__ import annotations

import json
import logging
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any
from urllib.parse import unquote, urlparse

LOG = logging.getLogger("e2e.identity-stub")

# The one person the stub resolves. `email` is the lookup key; the whole dict is
# the Person body the analytics get_person handler returns verbatim on 200. Its
# field set + names MUST match analytics `infra::identity::Person` (all
# snake_case; every non-Option field required) or the client's
# `resp.json::<Person>()` fails and the handler maps that to a 500 — not a 200.
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

_PREFIX = "/v1/persons/"


class _Handler(BaseHTTPRequestHandler):
    """Serves GET /v1/persons/{email}; the seeded map lives on `self.server`."""

    def do_GET(self) -> None:  # noqa: N802 — BaseHTTPRequestHandler API
        path = urlparse(self.path).path
        if not path.startswith(_PREFIX):
            self._send(404, {"error": "not found", "path": path})
            return
        email = unquote(path[len(_PREFIX) :])
        person = self.server.people.get(email)  # type: ignore[attr-defined]
        if person is None:
            self._send(404, {"error": "person not found", "email": email})
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
        self._thread = threading.Thread(
            target=server.serve_forever, name="identity-stub", daemon=True
        )
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
