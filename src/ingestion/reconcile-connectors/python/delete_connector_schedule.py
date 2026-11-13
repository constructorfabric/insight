from __future__ import annotations

import argparse
import os
import sys
from collections.abc import Mapping

from read_connector_schedules import ScheduleReadError
from schedule_database import connect
from schedule_lock import LOCK_TIMEOUT_SECS, instance_lock


def run(env: Mapping[str, str], connector: str, source_id: str) -> None:
    if not any(key.startswith("CONNECTOR_SCHEDULES_DB_") for key in env):
        return
    with (
        connect(env, lock_timeout_secs=LOCK_TIMEOUT_SECS) as connection,
        instance_lock(connection, connector, source_id),
        connection.cursor() as cursor,
    ):
        cursor.execute(
            "DELETE FROM connector_schedules WHERE connector = %s AND source_id = %s", (connector, source_id)
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--connector", required=True)
    parser.add_argument("--source-id", required=True)
    args = parser.parse_args()
    try:
        run(os.environ, args.connector, args.source_id)
    except Exception as error:
        reason = str(error) if isinstance(error, ScheduleReadError) else type(error).__name__
        print(f"schedule delete failed: {reason}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
