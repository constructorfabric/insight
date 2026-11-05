from __future__ import annotations

import json
import os
import unicodedata
from collections.abc import Mapping
from dataclasses import asdict, dataclass

from schedule_database import connect

MAX_ROWS = 10000
MAX_BYTES = 4 * 1024 * 1024


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
    return (
        isinstance(value, str)
        and 0 < len(value) <= 64
        and value.isascii()
        and not has_control(value)
        and not any(c.isspace() for c in value)
    )


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
        if not isinstance(crons, list) or not 0 <= len(crons) <= 10:
            raise ScheduleReadError("invalid schedule crons")
        if any(not isinstance(cron, str) or not cron.strip() or len(cron) > 256 or has_control(cron) for cron in crons):
            raise ScheduleReadError("invalid schedule cron string")
        if type(revision) is not int or not 1 <= revision <= 2**64 - 1:
            raise ScheduleReadError("invalid schedule revision")
        pair = (connector, source_id)
        if pair in seen:
            raise ScheduleReadError("duplicate schedule identity")
        seen.add(pair)
        schedules.append(Schedule(connector, source_id, tuple(crons), revision))
    return tuple(schedules)


def read_schedules(env: Mapping[str, str]) -> dict[str, object]:
    try:
        with connect(env) as connection, connection.cursor() as cursor:
            cursor.execute(
                "SELECT connector, source_id, revision, "
                "CASE WHEN OCTET_LENGTH(crons) <= %s THEN crons ELSE NULL END AS crons "
                "FROM connector_schedule_requests_v1 ORDER BY connector, source_id LIMIT %s",
                (4096, MAX_ROWS + 1),
            )
            rows = []
            size = 0
            for row in cursor:
                connector, source_id, revision, raw_crons = row
                size += len((raw_crons or "").encode("utf-8"))
                if len(rows) >= MAX_ROWS or size > MAX_BYTES:
                    raise ScheduleReadError("snapshot exceeds limit")
                rows.append(
                    dict(connector=connector, source_id=source_id, revision=revision, crons=json.loads(raw_crons))
                )
            schedules = validate_response({"version": 1, "schedules": rows})
        return {"status": "ok", "schedules": [asdict(schedule) for schedule in schedules]}
    except Exception:
        return {"status": "unavailable", "reason": "schedule database read failed"}


if __name__ == "__main__":
    print(json.dumps(read_schedules(os.environ)))
