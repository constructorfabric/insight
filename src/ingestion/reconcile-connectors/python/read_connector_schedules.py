from __future__ import annotations

import json
import os
import subprocess
import unicodedata
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlencode

REQUIRED_ENV = (
    "CONNECTOR_SCHEDULES_URL",
    "SERVICE_TOKEN_URL",
    "SERVICE_TOKEN_NAME",
    "SERVICE_TOKEN_KEY_FILE",
    "SERVICE_TOKEN_TENANT_ID",
)


class ScheduleReadError(Exception):
    pass


@dataclass(frozen=True)
class Schedule:
    connector: str
    source_id: str
    crons: tuple[str, ...]
    revision: int


def has_control(value: str) -> bool:
    return any(unicodedata.category(char) in {"Cc", "Cf", "Zl", "Zp"} for char in value)


def valid_identity(value: object) -> bool:
    return isinstance(value, str) and bool(value) and not has_control(value) and not any(c.isspace() for c in value)


def validate_response(body: object) -> tuple[Schedule, ...]:
    if not isinstance(body, dict) or type(body.get("version")) is not int or body["version"] != 1:
        raise ScheduleReadError("invalid schedules response version")
    entries = body.get("schedules")
    if not isinstance(entries, list):
        raise ScheduleReadError("invalid schedules response list")

    schedules = []
    seen = set()
    for entry in entries:
        if not isinstance(entry, dict):
            raise ScheduleReadError("invalid schedule entry")
        connector = entry.get("connector")
        source_id = entry.get("source_id")
        crons = entry.get("crons")
        revision = entry.get("revision")
        if not valid_identity(connector) or not valid_identity(source_id):
            raise ScheduleReadError("invalid schedule identity")
        if not isinstance(crons, list) or not 1 <= len(crons) <= 10:
            raise ScheduleReadError("invalid schedule crons")
        if any(not isinstance(cron, str) or not cron.strip() or has_control(cron) for cron in crons):
            raise ScheduleReadError("invalid schedule cron string")
        if type(revision) is not int:
            raise ScheduleReadError("invalid schedule revision")
        pair = (connector, source_id)
        if pair in seen:
            raise ScheduleReadError("duplicate schedule identity")
        seen.add(pair)
        schedules.append(Schedule(connector, source_id, tuple(crons), revision))
    return tuple(schedules)


def sign_assertion(env: Mapping[str, str]) -> str:
    try:
        result = subprocess.run(
            ["node", str(Path(__file__).with_name("sign_service_assertion.js"))],
            env=env,
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise ScheduleReadError("service assertion signing failed") from exc
    if result.returncode or not result.stdout:
        raise ScheduleReadError("service assertion signing failed")
    return result.stdout


def request_json(url: str, stage: str, payload: str, *, post: bool) -> object:
    command = [
        "curl",
        "--silent",
        "--show-error",
        "--connect-timeout",
        "5",
        "--max-time",
        "15",
        "--proto",
        "=http,https",
        "--write-out",
        "\n%{http_code}",
    ]
    if post:
        command.extend(["--header", "Content-Type: application/x-www-form-urlencoded", "--data-binary", "@-"])
    else:
        command.extend(["--header", "@-"])
    command.extend(["--url", url])
    try:
        result = subprocess.run(command, input=payload, capture_output=True, text=True, timeout=17, check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise ScheduleReadError(f"{stage} request failed") from exc
    if result.returncode:
        raise ScheduleReadError(f"{stage} request failed")
    body, _, status = result.stdout.rpartition("\n")
    if status != "200":
        raise ScheduleReadError(f"{stage} HTTP {status}" if status.isdigit() else f"{stage} request failed")
    try:
        return json.loads(body)
    except ValueError as exc:
        raise ScheduleReadError(f"{stage} returned invalid JSON") from exc


def exchange_token(env: Mapping[str, str]) -> object:
    assertion = sign_assertion(env)
    form = urlencode(
        {
            "grant_type": "client_credentials",
            "client_assertion_type": "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
            "client_assertion": assertion,
            "tenant_id": env["SERVICE_TOKEN_TENANT_ID"],
        }
    )
    return request_json(env["SERVICE_TOKEN_URL"], "token", form, post=True)


def read_schedules(env: Mapping[str, str]) -> dict[str, object]:
    if env.get("CONNECTOR_SCHEDULES_ENABLED") != "true":
        return {"status": "off"}
    try:
        for name in REQUIRED_ENV:
            if not env.get(name):
                raise ScheduleReadError(f"missing {name}")
        token_body = exchange_token(env)
        token = token_body.get("access_token") if isinstance(token_body, dict) else None
        if not isinstance(token, str) or not token or any(c.isspace() for c in token) or has_control(token):
            raise ScheduleReadError("token response has no valid access_token")
        body = request_json(env["CONNECTOR_SCHEDULES_URL"], "schedules", f"Authorization: Bearer {token}\n", post=False)
        schedules = validate_response(body)
        return {
            "status": "ok",
            "schedules": [
                {"connector": s.connector, "source_id": s.source_id, "crons": s.crons, "revision": s.revision}
                for s in schedules
            ],
        }
    except ScheduleReadError as exc:
        return {"status": "unavailable", "reason": str(exc)}


if __name__ == "__main__":
    print(json.dumps(read_schedules(os.environ)))
