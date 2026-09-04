"""The instance's ClickHouse over HTTP: seed it, read it back, clear it."""

from __future__ import annotations

import logging
from collections.abc import Sequence
from typing import Any

import clickhouse_connect
from clickhouse_connect.driver.client import Client

from insight_datapath.instance import InstanceConfig

LOG = logging.getLogger("datapath.clickhouse")


def client(cfg: InstanceConfig, *, database: str | None = None) -> Client:
    return clickhouse_connect.get_client(
        host=cfg.ch_host,
        port=cfg.ch_http_port,
        username=cfg.ch_user,
        password=cfg.ch_password,
        database=database or "default",
    )


def execute(cfg: InstanceConfig, sql: str, *, database: str | None = None) -> None:
    """Run a statement that returns no rows."""
    LOG.debug("ch exec: %s", sql.splitlines()[0][:120])
    with client(cfg, database=database) as connection:
        connection.command(sql)


def query(cfg: InstanceConfig, sql: str, *, database: str | None = None) -> list[Sequence[Any]]:
    with client(cfg, database=database) as connection:
        return list(connection.query(sql).result_rows)


def insert(
    cfg: InstanceConfig, table: str, rows: Sequence[Sequence[Any]], columns: Sequence[str]
) -> None:
    database, _, name = table.partition(".")
    with client(cfg, database=database) as connection:
        connection.insert(name, list(rows), column_names=list(columns))


def ensure_database(cfg: InstanceConfig, name: str) -> None:
    execute(cfg, f"CREATE DATABASE IF NOT EXISTS `{name}`")
