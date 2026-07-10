#!/usr/bin/env python3
"""Echo upstream for the gateway e2e (dev/CI only).

Reflects the request line and every received header back as JSON, so the e2e can
assert what the gateway forwarded upstream (the R3 poisoned-header proof):
  * Authorization is the injected `Bearer <gateway JWT>`, not the browser's,
  * the __Host-sid session cookie never arrives,
  * X-Correlation-Id is present and unique per request.

Bind address from argv[1] (default 0.0.0.0:9090).
"""

import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _echo(self):
        body = json.dumps(
            {
                "method": self.command,
                "path": self.path,
                # Header names are case-insensitive; report them lowercased so the
                # e2e can assert without worrying about casing.
                "headers": {k.lower(): v for k, v in self.headers.items()},
            }
        ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_GET = _echo
    do_POST = _echo
    do_PUT = _echo
    do_DELETE = _echo

    def log_message(self, *_args):  # silence access logging
        pass


if __name__ == "__main__":
    host, _, port = (sys.argv[1] if len(sys.argv) > 1 else "0.0.0.0:9090").partition(":")
    ThreadingHTTPServer((host, int(port)), Handler).serve_forever()
