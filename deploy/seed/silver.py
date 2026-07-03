"""
ClickHouse silver-layer schema bootstrap + sample-data generation.

Three responsibilities, run in order on every `silver` subcommand. All
three are idempotent — re-running converges on the same end state.

1. Create the bronze + silver placeholder tables that the gold-view
   migrations reference. Driven by `sql/placeholders.sql`, an extract
   of `insight/src/ingestion/scripts/create-bronze-placeholders.sh`
   (which is k8s-coupled and unusable from compose).

2. Apply the gold-view migrations from
   `insight/src/ingestion/scripts/migrations/*.sql` in lexicographic
   order. Migrations are `DROP VIEW IF EXISTS` + `CREATE VIEW`.

3. Generate per-team activity rows via `generators/*.py`. Volumes scale
   by team profile + persona; per-day caps live in each generator
   module.
"""

from __future__ import annotations

import logging
import os
import re
from pathlib import Path

import clickhouse_connect

from generators import ai, collab, crm, git, hr, people, support, task
from profiles import build_roster, get_dev_user_email

LOG = logging.getLogger("seed.silver")

DEFAULT_DAYS = 60

# SQL inputs. Defaults are the compose seed-sample bind-mount targets
# (see docker-compose.yml `seed-sample.volumes`); host runs override
# via env to point at the actual filesystem paths.
PLACEHOLDERS_SQL = Path(os.environ.get("PLACEHOLDERS_SQL") or "/app/sql/placeholders.sql")
MIGRATIONS_DIR   = Path(os.environ.get("MIGRATIONS_DIR")   or "/migrations")


def _ch_client() -> clickhouse_connect.driver.client.Client:
    host = os.environ.get("CLICKHOUSE_HOST", "clickhouse")
    port = int(os.environ.get("CLICKHOUSE_HTTP_PORT", "8123"))
    user = os.environ.get("CLICKHOUSE_USER", "insight")
    pwd = os.environ.get("CLICKHOUSE_PASSWORD", "insight-local")
    # CRITICAL: analytics queries with join_use_nulls=1, so views must
    # be CREATED with the same setting — otherwise the view's declared
    # column types disagree with what the query sees at runtime
    # ("Nullable column having not Nullable type in structure").
    return clickhouse_connect.get_client(
        host=host, port=port, username=user, password=pwd,
        settings={"join_use_nulls": 1},
    )


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
            "Did the seed-sample container mount /app/sql? "
            "(host runs: set PLACEHOLDERS_SQL to an existing "
            "placeholders.sql path, e.g. ./sql/placeholders.sql when "
            "running from deploy/seed.)"
        )
    n = _apply_sql_file(client, PLACEHOLDERS_SQL)
    LOG.info("placeholders: %d statements applied", n)
    return n


def apply_migrations(client: clickhouse_connect.driver.client.Client) -> int:
    """Apply gold-view migrations in lexicographic order."""
    if not MIGRATIONS_DIR.is_dir():
        raise FileNotFoundError(
            f"migrations dir not found at {MIGRATIONS_DIR}. "
            "Did the seed-sample container mount /migrations? "
            "(host runs: set MIGRATIONS_DIR to an existing migrations "
            "directory, e.g. ../../src/ingestion/scripts/migrations "
            "when running from deploy/seed.)"
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


def generate_rows(
    client: clickhouse_connect.driver.client.Client,
) -> None:
    """Populate silver tables with per-team activity for the demo roster."""
    tenant_uuid = os.environ.get(
        "TENANT_DEFAULT_ID", "00000000-df51-5b42-9538-d2b56b7ee953"
    )
    dev_email = get_dev_user_email()
    roster = build_roster(dev_email)
    days = int(os.environ.get("SEED_DAYS", DEFAULT_DAYS))
    LOG.info(
        "generating silver rows: tenant=%s days=%d persons=%d",
        tenant_uuid, days, len(roster),
    )

    totals: dict[str, int] = {}
    totals.update(people.generate(client, roster))
    totals.update(git.generate(client, roster, tenant_uuid, days))
    totals.update(crm.generate(client, roster, tenant_uuid, days))
    totals.update(collab.generate(client, roster, tenant_uuid, days))
    totals.update(hr.generate(client, roster, tenant_uuid, days))
    totals.update(ai.generate(client, roster, tenant_uuid, days))
    totals.update(task.generate(client, roster, tenant_uuid, days))
    totals.update(support.generate(client, roster, tenant_uuid, days))

    for table, n in sorted(totals.items()):
        LOG.info("  %-46s %6d rows", table, n)
    LOG.info("silver rows: %d total across %d tables",
             sum(totals.values()), len(totals))


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
        generate_rows(client)
        LOG.info("DONE: silver schema + gold views + sample rows in place.")
    finally:
        client.close()


if __name__ == "__main__":
    run()
