from __future__ import annotations

import argparse
import os
import sys
from collections.abc import Mapping
from pathlib import Path

from delete_connector_schedule import run as delete
from read_connector_schedules import ScheduleReadError
from schedule_database import connect


def secret_instances(secrets_tsv: str) -> set[tuple[str, str]]:
    instances = set()
    for line in secrets_tsv.splitlines():
        fields = line.split("\t")
        if len(fields) >= 3 and fields[0] and fields[2]:
            instances.add((fields[0], fields[1]))
    return instances


def run(env: Mapping[str, str], secrets_tsv: str, connector: str = "") -> int:
    if not any(key.startswith("CONNECTOR_SCHEDULES_DB_") for key in env):
        return 0
    configured = secret_instances(secrets_tsv)
    with connect(env) as connection, connection.cursor() as cursor:
        if connector:
            cursor.execute("SELECT connector, source_id FROM connector_schedules WHERE connector = %s", (connector,))
        else:
            cursor.execute("SELECT connector, source_id FROM connector_schedules")
        rows = cursor.fetchall()
    failures = 0
    for name, source_id in rows:
        if (name, source_id) in configured:
            continue
        try:
            delete(env, name, source_id)
        except Exception as error:
            reason = str(error) if isinstance(error, ScheduleReadError) else type(error).__name__
            print(f"schedule orphan delete failed for {name}/{source_id}: {reason}", file=sys.stderr)
            failures += 1
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("secrets", type=Path)
    parser.add_argument("--connector", default="")
    args = parser.parse_args()
    try:
        failures = run(os.environ, args.secrets.read_text(encoding="utf-8"), args.connector)
    except Exception as error:
        print(f"schedule orphan sweep failed: {type(error).__name__}", file=sys.stderr)
        failures = 1
    print(failures)
    return 0


if __name__ == "__main__":
    sys.exit(main())
