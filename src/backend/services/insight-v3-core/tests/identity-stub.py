#!/usr/bin/env python3
"""The identity answer the admin-only surfaces ask for.

`GET /v1/me` is the only call this service makes of identity: it forwards the
caller's authorization and reads the roles off the answer. The seeded admin
role id is a migration constant of the identity service, mirrored here — see
ADMIN_ROLE_ID in src/identity.rs.

Anything else 404s, so a test that starts calling a second endpoint fails
here rather than passing on a stub that answered everything.
"""

from __future__ import annotations

import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

ADMIN_ROLE_ID = "a4d11000-0000-4000-8000-000000000001"


class Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 - the base class names it
        if self.path.split("?")[0] != "/v1/me":
            self.send_error(404, "only /v1/me is stubbed")
            return

        # No authorization is no caller: the service must send the header it
        # was given, and a stub that ignored it would hide that.
        if not self.headers.get("authorization"):
            self.send_error(401, "no authorization forwarded")
            return

        body = json.dumps({"roles": [{"role_id": ADMIN_ROLE_ID}]}).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args: object) -> None:
        """Quiet: the test's own output is the record."""


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18099
    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
