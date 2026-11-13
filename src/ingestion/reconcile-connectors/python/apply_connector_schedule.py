from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from collections.abc import Mapping
from typing import TYPE_CHECKING

import yaml
from read_connector_schedules import ScheduleReadError, validate_response
from schedule_database import connect
from schedule_lock import LOCK_TIMEOUT_SECS, instance_lock

if TYPE_CHECKING:
    import pymysql


def apply(manifest: str) -> None:
    subprocess.run(["kubectl", "apply", "-f", "-"], input=manifest, text=True, check=True, timeout=60)


def render_schedule(manifest: str, crons: tuple[str, ...]) -> str:
    document = yaml.safe_load(manifest)
    if crons:
        document["spec"]["schedules"] = list(crons)
        document["metadata"]["annotations"]["insight.cyberfabric.com/schedule-origin"] = "api"
    return yaml.safe_dump(document, sort_keys=False)


def locked_apply(connection: pymysql.Connection, connector: str, source_id: str, manifest: str) -> None:
    with instance_lock(connection, connector, source_id), connection.cursor() as cursor:
        cursor.execute(
            "SELECT connector, source_id, revision, "
            "CASE WHEN OCTET_LENGTH(crons) <= 4096 THEN crons ELSE NULL END "
            "FROM connector_schedule_requests_v1 WHERE connector = %s AND source_id = %s",
            (connector, source_id),
        )
        rows = cursor.fetchall()
        entries = [dict(connector=c, source_id=s, revision=r, crons=json.loads(raw)) for c, s, r, raw in rows]
        schedules = validate_response({"version": 1, "schedules": entries})
        crons = schedules[0].crons if schedules else ()
        apply(render_schedule(manifest, crons))


def run(env: Mapping[str, str], connector: str, source_id: str, manifest: str) -> None:
    if not any(key.startswith("CONNECTOR_SCHEDULES_DB_") for key in env):
        apply(manifest)
        return
    with connect(env, lock_timeout_secs=LOCK_TIMEOUT_SECS) as connection:
        locked_apply(connection, connector, source_id, manifest)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--connector", required=True)
    parser.add_argument("--source-id", required=True)
    args = parser.parse_args()
    try:
        run(os.environ, args.connector, args.source_id, sys.stdin.read())
    except Exception as error:
        reason = str(error) if isinstance(error, ScheduleReadError) else type(error).__name__
        print(f"schedule apply failed: {reason}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
