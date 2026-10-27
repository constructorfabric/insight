#!/usr/bin/env python3
# ruff: noqa: T201 — stdout/stderr are this script's CI report.
from __future__ import annotations

import argparse
import os
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from uuid import uuid4

import httpx
from insight_stand.errors import StandError
from insight_stand.personas import persona_password
from insight_stand.session import LoginSession


@dataclass(frozen=True)
class Options:
    base_url: str
    realm_path: Path
    admin_email: str
    ingest_token: str = field(repr=False)


class SmokeFailure(Exception):
    pass


def unexpected_response(response: httpx.Response, reason: str) -> SmokeFailure:
    request = response.request
    print(f"request: {request.method} {request.url.copy_with(query=None)}", file=sys.stderr)
    if request.url.path.startswith("/api/core/") and request.content:
        print(f"request body: {request.content.decode()}", file=sys.stderr)
    print(f"status: {response.status_code}", file=sys.stderr)
    print(f"response body: {response.text}", file=sys.stderr)
    return SmokeFailure(reason)


def request(
    client: httpx.Client,
    method: str,
    path: str,
    expected_status: int,
    headers: dict[str, str],
    body: dict[str, object] | None = None,
    timeout: float = 30.0,
) -> httpx.Response:
    response = client.request(method, path, headers=headers, json=body, timeout=timeout)
    if response.status_code != expected_status:
        raise unexpected_response(response, f"expected HTTP {expected_status}")
    return response


def json_object(response: httpx.Response) -> dict[str, object]:
    try:
        payload: object = response.json()
    except ValueError as error:
        raise unexpected_response(response, "expected a JSON object") from error
    if not isinstance(payload, dict):
        raise unexpected_response(response, "expected a JSON object")
    return payload


def wait_for_record(
    client: httpx.Client, path: str, admin_headers: dict[str, str], raw_data: dict[str, object]
) -> None:
    deadline = time.monotonic() + 30.0
    while True:
        remaining = max(0.1, deadline - time.monotonic())
        response = request(client, "GET", path, 200, admin_headers, timeout=min(10.0, remaining))
        page = json_object(response)
        records = page.get("records")
        if not isinstance(records, list) or page.get("total") not in (0, 1):
            raise unexpected_response(response, "expected a page holding at most one record")
        if records and (
            len(records) != 1 or not isinstance(records[0], dict) or records[0].get("raw_data") != raw_data
        ):
            raise unexpected_response(response, "the record differs from the ingested payload")
        if len(records) == 1 and page["total"] == 1:
            return

        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise unexpected_response(response, "the record did not become visible within 30 seconds")
        time.sleep(min(0.5, remaining))


def run(options: Options) -> None:
    session = LoginSession(
        base_url=options.base_url,
        email=options.admin_email,
        password=persona_password(options.admin_email, realm_path=options.realm_path),
    )
    admin_headers = session.headers()
    name = f"functional_ci_{uuid4().hex}"
    path = f"/api/core/v1/datasets/{name}"
    declaration: dict[str, object] = {
        "title": "Synthetic smoke events",
        "fields": [{"name": "kind", "path": "kind", "type": "string"}],
    }
    raw_data: dict[str, object] = {"kind": "functional-ci", "value": 42}

    with httpx.Client(base_url=options.base_url, timeout=30.0, follow_redirects=False) as client:
        declared = request(client, "PUT", path, 200, admin_headers, declaration)
        try:
            if json_object(declared).get("name") != name:
                raise unexpected_response(declared, "the declaration names a different dataset")
            request(
                client,
                "POST",
                "/api/core/v1/raw-data",
                204,
                {"X-Insight-Token": options.ingest_token},
                {"dataset": name, "raw_data": raw_data},
            )
            wait_for_record(client, f"{path}/records", admin_headers, raw_data)
        finally:
            request(client, "DELETE", path, 204, admin_headers)

    print("raw-data lifecycle: declared, ingested, read back, and deleted")


def parse_args() -> Options:
    parser = argparse.ArgumentParser(description="Smoke the raw-data lifecycle through the real OIDC chain")
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--realm-path", required=True, type=Path)
    parser.add_argument("--admin-email", required=True)
    parser.add_argument("--ingest-token", default=os.environ.get("INSIGHT_SMOKE_INGEST_TOKEN"))
    args = parser.parse_args()
    if not args.ingest_token:
        parser.error("--ingest-token or INSIGHT_SMOKE_INGEST_TOKEN is required")
    return Options(args.base_url, args.realm_path, args.admin_email, args.ingest_token)


def main() -> int:
    options = parse_args()
    try:
        run(options)
    except httpx.RequestError as error:
        print(f"request: {error.request.method} {error.request.url.copy_with(query=None)}", file=sys.stderr)
        print("status: unavailable\nresponse body: unavailable", file=sys.stderr)
        print(f"error: {error}", file=sys.stderr)
        return 1
    except (SmokeFailure, StandError, OSError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
