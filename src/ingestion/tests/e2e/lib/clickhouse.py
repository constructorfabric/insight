"""ClickHouse HTTP client used by migration-applier and (later) ch-seeder."""

from __future__ import annotations

import logging
from typing import Any

import clickhouse_connect

from lib.config import SessionConfig

LOG = logging.getLogger("e2e.ch")


def client(cfg: SessionConfig, *, database: str | None = None):
    """Return a clickhouse_connect HTTP client bound to the session's CH.

    Refreshable materialized views are GA on the pinned ClickHouse (25.x),
    so no experimental settings are needed here anymore.
    """
    return clickhouse_connect.get_client(
        host=cfg.ch_host,
        port=cfg.ch_http_port,
        username=cfg.ch_user,
        password=cfg.ch_password,
        database=database or "default",
    )


def execute(cfg: SessionConfig, sql: str, *, database: str | None = None) -> None:
    """Run a statement that returns no rows (DDL, INSERT, TRUNCATE…)."""
    LOG.debug("ch exec: %s", sql.splitlines()[0][:120])
    with client(cfg, database=database) as c:
        c.command(sql)


def query(cfg: SessionConfig, sql: str, *, database: str | None = None) -> list[tuple[Any, ...]]:
    """Run a SELECT and return rows as tuples."""
    with client(cfg, database=database) as c:
        return c.query(sql).result_rows


def ensure_database(cfg: SessionConfig, name: str) -> None:
    """CREATE DATABASE IF NOT EXISTS — used by session setup before migrations."""
    execute(cfg, f"CREATE DATABASE IF NOT EXISTS {name}")
