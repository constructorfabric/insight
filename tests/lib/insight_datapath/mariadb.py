"""The instance's MariaDB: the identity state a spec's people are minted into."""

from __future__ import annotations

from collections.abc import Sequence
from typing import Any

import pymysql
from pymysql.connections import Connection

from insight_datapath.instance import InstanceConfig


def connection(cfg: InstanceConfig, *, database: str | None = None) -> Connection:
    """A connection the caller closes."""
    return pymysql.connect(
        host=cfg.mariadb_host,
        port=cfg.mariadb_port,
        user=cfg.mariadb_user,
        password=cfg.mariadb_password,
        database=database or cfg.mariadb_database,
        charset="utf8mb4",
        autocommit=True,
    )


def query(
    cfg: InstanceConfig,
    sql: str,
    parameters: Sequence[Any] = (),
    *,
    database: str | None = None,
) -> list[tuple[Any, ...]]:
    with connection(cfg, database=database) as conn, conn.cursor() as cursor:
        cursor.execute(sql, tuple(parameters))
        return list(cursor.fetchall())


def execute(
    cfg: InstanceConfig,
    sql: str,
    parameters: Sequence[Any] = (),
    *,
    database: str | None = None,
) -> int:
    """Run a statement and return how many rows it changed."""
    with connection(cfg, database=database) as conn, conn.cursor() as cursor:
        return cursor.execute(sql, tuple(parameters))
