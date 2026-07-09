"""MariaDB connection helper.

Used to verify MariaDB readiness in smoke tests and (later) inspect/seed the
`metrics` table that analytics consumes.
"""

from __future__ import annotations

import logging
import time
from typing import Any

import pymysql

from lib.config import SessionConfig

LOG = logging.getLogger("e2e.mariadb")


def connection(cfg: SessionConfig, *, database: str | None = None):
    """Return a pymysql connection (caller is responsible for closing)."""
    return pymysql.connect(
        host=cfg.mariadb_host,
        port=cfg.mariadb_port,
        user=cfg.mariadb_user,
        password=cfg.mariadb_password,
        database=database or cfg.mariadb_database,
        charset="utf8mb4",
        autocommit=True,
    )


def query(cfg: SessionConfig, sql: str, *, database: str | None = None) -> list[tuple[Any, ...]]:
    with connection(cfg, database=database) as conn:
        with conn.cursor() as cur:
            cur.execute(sql)
            return list(cur.fetchall())


def wait_ready(cfg: SessionConfig, *, timeout_s: float = 60.0) -> None:
    """Sometimes the container reports healthy before SELECT 1 succeeds."""
    deadline = time.monotonic() + timeout_s
    last_err: Exception | None = None
    while time.monotonic() < deadline:
        try:
            with connection(cfg) as conn:
                with conn.cursor() as cur:
                    cur.execute("SELECT 1")
                    cur.fetchone()
            return
        except Exception as e:
            last_err = e
            time.sleep(1.0)
    raise RuntimeError(f"MariaDB not ready in {timeout_s}s: {last_err}")
