#!/usr/bin/env python3
"""Minimal Identity stub for the authenticator e2e runner (dev/CI only).

Answers the authenticator's internal identity lookups (mirrors
identity-resolution's real handlers):

- `GET /internal/persons/by-external-id?source_type=...&external_id=...`
  (login bootstrap)
- `GET /internal/persons/by-email-override?email=...` (admin `__override`)
- `GET /internal/persons/active-roles?person_id=...` (live authorization)

One test seam beyond the lookups: `POST /__test/roles-stall?person_id=P&seconds=N`
arms a one-shot stall on the next active-roles call FOR THAT PERSON, so an e2e
can hold Identity open the way a restarting one does. Addressed by person so a
suite running its tests concurrently cannot consume another's stall, and so a
stall left armed by a failed test dies with its own dedicated user.

Each answers with a deterministic `insight_source_id`, so the login loop and
the `__override` view-as loop can resolve a person without standing up the
real identity-resolution service + seeding. The real endpoints gate on a
service gateway JWT; the stub ignores the bearer (test seam). Any other path
404s. Bind address from argv[1] (default 127.0.0.1:8092).
"""

import hashlib
import json
import sys
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit


def person_id_for(*parts: str) -> str:
    # Deterministic UUID from the lookup key (stable across calls within a run).
    digest = hashlib.sha256(f"identity-stub:{':'.join(parts)}".encode()).digest()
    return str(uuid.UUID(bytes=digest[:16]))


# person_id -> seconds its next active-roles call sleeps; consumed by that call.
_roles_stalls: dict[str, float] = {}
_stall_lock = threading.Lock()


def take_roles_stall(person_id: str) -> float:
    with _stall_lock:
        return _roles_stalls.pop(person_id, 0.0)


class Handler(BaseHTTPRequestHandler):
    BY_EXTERNAL_ID_PATH = "/internal/persons/by-external-id"
    BY_ROSTER_EMAIL_PATH = "/internal/persons/by-roster-email"
    BY_EMAIL_OVERRIDE_PATH = "/internal/persons/by-email-override"
    ACTIVE_ROLES_PATH = "/internal/persons/active-roles"
    ROLES_STALL_PATH = "/__test/roles-stall"

    def do_POST(self):  # noqa: N802
        split = urlsplit(self.path)
        if split.path != self.ROLES_STALL_PATH:
            # 501, not 404: a 404 is how the lookups say "no such person", and
            # the authenticator reads it that way (see provision's INVARIANT).
            self.send_response(501)
            self.end_headers()
            return
        query = parse_qs(split.query)
        person_id = (query.get("person_id") or [""])[0]
        try:
            seconds = max(0.0, float((query.get("seconds") or ["0"])[0]))
        except ValueError:
            seconds = -1.0
        if not person_id or seconds < 0:
            self.send_response(400)
            self.end_headers()
            return
        with _stall_lock:
            _roles_stalls[person_id] = seconds
        self.send_response(204)
        self.end_headers()

    def do_GET(self):  # noqa: N802
        split = urlsplit(self.path)
        query = parse_qs(split.query)

        if split.path == self.ACTIVE_ROLES_PATH:
            person_id = (query.get("person_id") or [""])[0]
            if not person_id:
                self.send_response(400)
                self.end_headers()
                return
            time.sleep(take_roles_stall(person_id))
            body = json.dumps({"roles": ["user", "admin"]}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return

        if split.path == self.BY_EXTERNAL_ID_PATH:
            source_type = (query.get("source_type") or [""])[0]
            external_id = (query.get("external_id") or [""])[0]
            if not source_type or not external_id:
                self.send_response(400)
                self.end_headers()
                return
            # The rig configures `idp.external_id_claim: email`, so a login's
            # external id IS the email — keyed identically to the `__override`
            # email lookup, both paths resolve the same user to the SAME
            # person (a real identity-resolution service converges the same
            # way: one `persons` person_id, two value_type observations). A
            # non-email external id still gets a deterministic path-specific id.
            key = ("email", external_id) if "@" in external_id else ("id", source_type, external_id)
            value_type, value = "id", external_id
        elif split.path == self.BY_ROSTER_EMAIL_PATH:
            # The login bootstrap of an install running `idp.resolve_by: email`.
            # Keyed on the address like the override path, so a rig that logs in
            # through either resolves the same person — the real service does
            # too, from the same `value_type='email'` observations. What it does
            # NOT model is the confinement (tenant, roster source, live account):
            # those are the database's answer and belong to identity's own live
            # tests, not to a stub whose job is to keep the authenticator's e2e
            # honest about which route it called.
            email = (query.get("email") or [""])[0]
            if not email:
                self.send_response(400)
                self.end_headers()
                return
            key = ("email", email)
            value_type, value = "email", email
        elif split.path == self.BY_EMAIL_OVERRIDE_PATH:
            email = (query.get("email") or [""])[0]
            if not email:
                self.send_response(400)
                self.end_headers()
                return
            key = ("email", email)
            value_type, value = "email", email
        else:
            self.send_response(404)
            self.end_headers()
            return

        # Test seam for the unknown-person paths (e.g. a bad `__override`
        # target): values prefixed `unknown-` do not resolve.
        if value.startswith("unknown-"):
            self.send_response(404)
            self.end_headers()
            return

        body = json.dumps(
            {
                "value_type": value_type,
                "value": value,
                "insight_source_type": "person",
                "insight_source_id": person_id_for(*key),
            }
        ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):  # silence access logging
        pass


if __name__ == "__main__":
    host, _, port = (sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1:8092").partition(":")
    # Threading: an armed stall holds one connection open, and the rest of the
    # rig must keep being served while it does.
    ThreadingHTTPServer((host, int(port)), Handler).serve_forever()
