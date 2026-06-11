"""
ClickHouse silver-layer schema bootstrap.

Two responsibilities, both idempotent:

1. Create the bronze + silver placeholder tables that the gold-view
   migrations reference. Driven by `sql/placeholders.sql`, an extract
   of `insight/src/ingestion/scripts/create-bronze-placeholders.sh`
   (which is k8s-coupled and unusable from compose).

2. Apply the gold-view migrations from
   `insight/src/ingestion/scripts/migrations/*.sql` in lexicographic
   order. These migrations are themselves idempotent
   (`DROP VIEW IF EXISTS` + `CREATE VIEW`) so re-runs are safe.

A Phase 3 commit will add the per-team row generators that INSERT into
the silver tables created here. For now this module only handles schema.
"""

from __future__ import annotations

import logging
import os
import re
from pathlib import Path

import clickhouse_connect

LOG = logging.getLogger("seed.silver")

# Bind-mount targets inside the seed-sample container — see
# docker-compose.yml `seed-sample.volumes`.
PLACEHOLDERS_SQL = Path("/app/sql/placeholders.sql")
MIGRATIONS_DIR = Path("/migrations")


def _ch_client() -> clickhouse_connect.driver.client.Client:
    host = os.environ.get("CLICKHOUSE_HOST", "clickhouse")
    port = int(os.environ.get("CLICKHOUSE_HTTP_PORT", "8123"))
    user = os.environ.get("CLICKHOUSE_USER", "insight")
    pwd = os.environ.get("CLICKHOUSE_PASSWORD", "insight-local")
    return clickhouse_connect.get_client(host=host, port=port, username=user, password=pwd)


_FULL_LINE_COMMENT = re.compile(r"^\s*--.*$", re.MULTILINE)


def _split_statements(sql: str) -> list[str]:
    """Split a multi-statement SQL block on `;` boundaries.

    Mirrors the init.sh sed pass that drops full-line `--` comments
    before piping into clickhouse-client. We do the same so a migration
    starting with a 20-line preamble doesn't choke the parser. Inline
    `-- foo` after SQL is left alone — those rarely break CH.
    """
    cleaned = _FULL_LINE_COMMENT.sub("", sql)
    return [stmt.strip() for stmt in cleaned.split(";") if stmt.strip()]


def _apply_sql_file(client: clickhouse_connect.driver.client.Client, path: Path) -> int:
    """Apply one SQL file. Returns the number of statements executed."""
    sql = path.read_text(encoding="utf-8")
    statements = _split_statements(sql)
    for stmt in statements:
        client.command(stmt)
    return len(statements)


def apply_placeholders(client: clickhouse_connect.driver.client.Client) -> int:
    """CREATE DATABASE + bronze/silver placeholder tables."""
    if not PLACEHOLDERS_SQL.is_file():
        raise FileNotFoundError(
            f"placeholders SQL not found at {PLACEHOLDERS_SQL}. "
            "Did the seed-sample container mount /app/sql?"
        )
    n = _apply_sql_file(client, PLACEHOLDERS_SQL)
    LOG.info("placeholders: %d statements applied", n)
    return n


def apply_migrations(client: clickhouse_connect.driver.client.Client) -> int:
    """Apply gold-view migrations in lexicographic order."""
    if not MIGRATIONS_DIR.is_dir():
        raise FileNotFoundError(
            f"migrations dir not found at {MIGRATIONS_DIR}. "
            "Did the seed-sample container mount /migrations?"
        )
    migrations = sorted(MIGRATIONS_DIR.glob("*.sql"))
    if not migrations:
        raise FileNotFoundError(f"no *.sql migrations under {MIGRATIONS_DIR}")
    total = 0
    for m in migrations:
        n = _apply_sql_file(client, m)
        LOG.info("migration %s: %d statements", m.name, n)
        total += n
    LOG.info("migrations: %d files applied, %d statements total", len(migrations), total)
    return total


def run() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
    )
    client = _ch_client()
    try:
        LOG.info("ClickHouse version: %s", client.server_version)
        apply_placeholders(client)
        apply_migrations(client)
        LOG.info("DONE: silver schema + gold views are in place.")
    finally:
        client.close()


if __name__ == "__main__":
    run()
