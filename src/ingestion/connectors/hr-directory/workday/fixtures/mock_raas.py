#!/usr/bin/env python3
"""Mock Workday RaaS server for local connector development.

Replays fixture responses in the RaaS shape ({"Report_Entry": [...]}) and
enforces the parts of the RaaS protocol the connector relies on:

- HTTP Basic auth header must be present (any credentials accepted)
- `format=json` query parameter is required
- the leave report requires `From_Date` and `To_Date` parameters, mirroring
  a report whose prompts are enabled as web service parameters

Routing is by report name (last path segment):
- .../<owner>/Insight_Employee_Sync -> workers.json
- .../<owner>/Insight_Leave_Sync    -> leave_requests.json

Usage:
    python3 mock_raas.py [--port 8765]

Point the connector at it via the K8s Secret file:
    workday_base_url: "http://host.docker.internal:<port>/ccx/service/customreport2/acme"
"""

from __future__ import annotations

import argparse
import json
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

FIXTURES_DIR = Path(__file__).resolve().parent

REPORT_FIXTURES = {
    "Insight_Employee_Sync": "workers.json",
    "Insight_Leave_Sync": "leave_requests.json",
}

PROMPT_REQUIRED = {
    "Insight_Leave_Sync": ("From_Date", "To_Date"),
}


class MockRaasHandler(BaseHTTPRequestHandler):
    def _reply(self, status: int, payload: dict) -> None:
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802 - http.server API
        parsed = urlparse(self.path)
        params = parse_qs(parsed.query)
        report = parsed.path.rstrip("/").rsplit("/", 1)[-1]

        auth = self.headers.get("Authorization", "")
        if not auth.lower().startswith("basic "):
            self._reply(401, {"error": "Basic authentication required"})
            return

        if report not in REPORT_FIXTURES:
            self._reply(404, {"error": f"Unknown report: {report}"})
            return

        if params.get("format") != ["json"]:
            self._reply(400, {"error": "format=json query parameter is required"})
            return

        missing = [p for p in PROMPT_REQUIRED.get(report, ()) if p not in params]
        if missing:
            # Real RaaS rejects parameters that do not match report prompts and
            # runs unfiltered when prompts are omitted; the mock is stricter so
            # a manifest that stops sending the prompts fails loudly in tests.
            self._reply(400, {"error": f"Missing prompt parameters: {missing}"})
            return

        fixture = FIXTURES_DIR / REPORT_FIXTURES[report]
        self._reply(200, json.loads(fixture.read_text()))

    def log_message(self, fmt: str, *args) -> None:
        print(f"[mock-raas] {self.address_string()} {fmt % args}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args()

    server = HTTPServer(("0.0.0.0", args.port), MockRaasHandler)
    print(f"[mock-raas] serving fixtures from {FIXTURES_DIR} on :{args.port}")
    server.serve_forever()


if __name__ == "__main__":
    main()
