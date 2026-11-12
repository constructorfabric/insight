from __future__ import annotations

import sys
from collections.abc import Iterator
from contextlib import contextmanager
from typing import TYPE_CHECKING

from read_connector_schedules import ScheduleReadError

if TYPE_CHECKING:
    import pymysql

LOCK_TIMEOUT_SECS = 60


@contextmanager
def instance_lock(connection: pymysql.Connection, connector: str, source_id: str) -> Iterator[None]:
    with connection.cursor() as cursor:
        cursor.execute(
            "SELECT GET_LOCK(SHA2(CONCAT(DATABASE(), '/', %s), 256), %s)",
            (f"{connector}/{source_id}", LOCK_TIMEOUT_SECS),
        )
        if cursor.fetchall() != [(1,)]:
            raise ScheduleReadError("schedule instance lock is busy")
        try:
            yield
        finally:
            try:
                cursor.execute(
                    "SELECT RELEASE_LOCK(SHA2(CONCAT(DATABASE(), '/', %s), 256))", (f"{connector}/{source_id}",)
                )
                if cursor.fetchall() != [(1,)]:
                    raise ScheduleReadError("schedule instance lock could not be released")
            except Exception as error:
                print(f"schedule lock release failed: {type(error).__name__}", file=sys.stderr)
