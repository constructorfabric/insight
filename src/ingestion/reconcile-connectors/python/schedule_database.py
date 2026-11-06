from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    import pymysql


def setting(env: Mapping[str, str], name: str) -> str:
    path = env.get(f"{name}_FILE")
    value = Path(path).read_text().rstrip("\r\n") if path else env.get(name, "")
    if not value:
        raise ValueError("missing database configuration")
    return value


def connect(env: Mapping[str, str]) -> pymysql.Connection:
    import pymysql

    return pymysql.connect(
        host=setting(env, "CONNECTOR_SCHEDULES_DB_HOST"),
        port=int(env.get("CONNECTOR_SCHEDULES_DB_PORT", "3306")),
        user=setting(env, "CONNECTOR_SCHEDULES_DB_USER"),
        password=setting(env, "CONNECTOR_SCHEDULES_DB_PASSWORD"),
        database=setting(env, "CONNECTOR_SCHEDULES_DB_NAME"),
        connect_timeout=5,
        read_timeout=15,
        write_timeout=15,
        charset="utf8mb4",
        cursorclass=pymysql.cursors.SSCursor,
        init_command="SET SESSION max_statement_time=10",
        autocommit=True,
    )
