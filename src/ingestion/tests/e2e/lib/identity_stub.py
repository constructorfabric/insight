"""In-process Identity stub for the bronze-to-api e2e rig (#1691).

A minimal loopback HTTP backend the analytics identity fan-outs resolve against:

- `POST {identity_url}/v1/profiles` — a canned profile for a seeded person
  (→ 200) and 404 for every other, so the persons endpoint exercises its real
  200/404 contract. Both key forms are served: `{value_type:"person_id",
  value:<uuid>}`, which is what the analytics persons facade sends since the
  identity cutover, and `{value_type:"email", value:<email>}`.
- `POST {identity_url}/v1/visible-persons` — the person UUIDs the caller may
  see (the identity-cutover contract), which the metric-results authorization
  gate compares against the requested ids. The reply is the intersection of
  the request with the fixture personas' derived UUIDs (`person_id_for`), so
  the metric suite's requests resolve as authorized and anyone else is
  refused.

Resolves purely by the request `value` and ignores headers on purpose. Analytics
forwards the caller's gateway JWT (Authorization) on this hop (NGINX_BFF G1), but
the stub does not verify it — the REAL Identity service would (R1); the stub is
what keeps this a test-only backend.
"""

from __future__ import annotations

import json
import logging
import threading
import uuid as _uuid
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

# The metric fixtures' personas — the set the stub reports as visible, so
# `POST /v1/metric-results` authorizes requests for them; an email outside this
# set is refused by the gate. The metric rig does NOT rely on this tuple: it
# calls `IdentityStub.allow_visible()` with the personas its yaml addresses, so
# a new fixture persona cannot fail as a phantom 403. The tuple remains the
# default for suites that spawn the stub without a yaml (api/…).
VISIBLE_EMAILS: tuple[str, ...] = (
    SEEDED_EMAIL,
    "alice@example.com",
    "bob@example.com",
    "carol@example.com",
    "erin@example.com",
    "frank@example.com",
    "grace@example.com",
)

# Deterministic persona UUIDs: uuid5 over the lowercased email in a fixed
# namespace. One derivation shared by the stub's visible set, the metric
# rig's email→person_id request translation and its identity_persons
# seeding — so the same yaml persona always resolves to the same person.
_PERSONA_NAMESPACE = _uuid.UUID("6e2e0000-0000-4000-8000-000000000001")


def person_id_for(email: str) -> str:
    """Canonical person UUID of a fixture persona (uuid5 of the email)."""
    return str(_uuid.uuid5(_PERSONA_NAMESPACE, email.strip().lower()))


# The seeded person's canonical id, and one that resolves to nobody: the
# analytics persons facade is keyed by person_id since the identity cutover.
SEEDED_PERSON_ID = person_id_for(SEEDED_EMAIL)
UNKNOWN_PERSON_ID = "019e2810-0000-7000-8000-0000000000ff"


_PROFILES_PATH = "/v1/profiles"
_VISIBLE_PERSONS_PATH = "/v1/visible-persons"


class _Handler(BaseHTTPRequestHandler):
    """Serves the two POST routes analytics calls; state lives on `self.server`."""

    def do_POST(self) -> None:
        path = urlparse(self.path).path
        if path not in (_PROFILES_PATH, _VISIBLE_PERSONS_PATH):
            self._send(404, {"error": "not found", "path": path})
            return

        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length) if length else b""
        try:
            body = json.loads(raw) if raw else {}
        except json.JSONDecodeError:
            self._send(400, {"error": "invalid json body"})
            return

        if path == _VISIBLE_PERSONS_PATH:
            self._send_visible(body)
        else:
            self._send_profile(body)

    def _send_visible(self, body: dict[str, Any]) -> None:
        # Person UUIDs since the identity cutover (the gate forwards the
        # validated request ids verbatim).
        requested = body.get("person_ids") or []
        visible = {person_id_for(email) for email in self.server.visible}  # type: ignore[attr-defined]
        self._send(200, {"visible": [p for p in requested if isinstance(p, str) and p.lower() in visible]})

    def _send_profile(self, body: dict[str, Any]) -> None:
        value = body.get("value", "")
        people: dict[str, dict[str, Any]] = self.server.people  # type: ignore[attr-defined]
        if body.get("value_type") == "person_id":
            key = {person_id_for(email): email for email in people}.get(str(value).lower())
            person = people.get(key) if key else None
        else:
            person = people.get(value)

        if person is None:
            self._send(404, {"error": "person not found", "value": value})
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

    def __init__(
        self, people: dict[str, dict[str, Any]] | None = None, visible: tuple[str, ...] = VISIBLE_EMAILS
    ) -> None:
        self._people = dict(people) if people is not None else {SEEDED_EMAIL: SEEDED_PERSON}
        self._visible = visible
        self._server: ThreadingHTTPServer | None = None
        self._thread: threading.Thread | None = None

    def start(self) -> None:
        # Port 0 → the OS assigns a free loopback port; read it back off the
        # bound socket (no find-free-port race).
        server = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
        server.people = self._people  # type: ignore[attr-defined]
        server.visible = self._visible  # type: ignore[attr-defined]
        self._server = server
        self._thread = threading.Thread(target=server.serve_forever, name="identity-stub", daemon=True)
        self._thread.start()
        LOG.info("identity stub listening on %s", self.url)

    def allow_visible(self, emails: tuple[str, ...] | list[str]) -> None:
        """Replace the visible set at runtime (the metric rig passes the
        personas its yaml addresses, so a new fixture persona is never a
        phantom 403 from a stale hand-maintained list)."""
        self._visible = tuple(emails)
        if self._server is not None:
            self._server.visible = self._visible  # type: ignore[attr-defined]

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
