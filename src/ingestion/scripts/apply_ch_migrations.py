#!/usr/bin/env python3
"""Apply the ClickHouse schema (dbt databases + bronze placeholders + gold-view
migrations) to an EXTERNAL ClickHouse over HTTP, driven entirely by env.

Why this exists
---------------
`scripts/init.sh` applies the same SQL via ``kubectl exec`` into a *bundled*
``statefulset/insight-clickhouse`` pod. When ClickHouse is external / layered
— e.g. a separate ``clickhouse`` Helm release in another namespace
(``clickhouse.insight-infra.svc``) or a managed warehouse — there is no
in-namespace pod to exec into, and the toolbox image ships no
``clickhouse-client`` binary. So the gold-view migrations never reach the live
CH and the views drift (observed: the Collaboration ``collab_bullet_rows`` view
stayed on its pre-deploy definition for >40h after CH moved to ``insight-infra``,
so every newly-released CH migration silently never applied).

This script talks to ``CLICKHOUSE_HOST`` over the HTTP interface using
``clickhouse-connect`` — already present in the toolbox image as a
``dbt-clickhouse`` dependency, so no new binary or image change. It is the same
transport + statement-splitting the e2e rig uses to apply these exact files
(``tests/e2e/e2e_lib/migration_applier.py``), so the behaviour is proven.

It is invoked by the ``clickhouse-migrate`` Helm hook Job on every
install/upgrade. Idempotent: ``CREATE DATABASE IF NOT EXISTS``,
``CREATE TABLE IF NOT EXISTS`` placeholders, and ``DROP VIEW IF EXISTS`` +
``CREATE VIEW`` migrations.

Env (all required unless noted):
  CLICKHOUSE_HOST        external CH host (FQDN)
  CLICKHOUSE_USER
  CLICKHOUSE_PASSWORD
  CLICKHOUSE_DATABASE    app database name (the dbt ``insight`` db)
  CLICKHOUSE_HTTP_PORT   default 8123
  CLICKHOUSE_PROTOCOL    http|https (default http) — https => secure connection
"""
from __future__ import annotations

import os
import re
import sys
import time
from pathlib import Path

import clickhouse_connect

SCRIPT_DIR = Path(__file__).resolve().parent
MIGRATIONS_DIR = SCRIPT_DIR / "migrations"
PLACEHOLDERS_SH = SCRIPT_DIR / "create-bronze-placeholders.sh"

_COMMENT_LINE = re.compile(r"^\s*--.*$", re.MULTILINE)


def _split_statements(sql: str) -> list[str]:
    """Strip SQL line-comments and split on `;`.

    ClickHouse migration files in this repo do not use string literals
    containing `;` or stored procedures, so a naive split is safe (same
    assumption as the e2e rig's `_split_statements`).
    """
    stripped = _COMMENT_LINE.sub("", sql)
    return [p.strip() for p in stripped.split(";") if p.strip()]


def _extract_heredoc_sql(bash_source: str) -> list[str]:
    """Pull the body of every ``run_ch <<'SQL' ... SQL`` heredoc, then split.

    Mirrors `tests/e2e/e2e_lib/migration_applier.py::_extract_heredoc_sql` so the
    bronze placeholders are applied from the SAME single source of truth as
    bundled init.sh and the e2e rig.
    """
    parts: list[str] = []
    buf: list[str] | None = None
    for line in bash_source.splitlines():
        if buf is None:
            if re.match(r"\s*run_ch\s*<<'?SQL'?\s*$", line):
                buf = []
            continue
        if line.strip() == "SQL":
            parts.append("\n".join(buf))
            buf = None
            continue
        buf.append(line)
    statements: list[str] = []
    for part in parts:
        statements.extend(_split_statements(part))
    return statements


def _client():
    host = os.environ["CLICKHOUSE_HOST"]
    port = int(os.environ.get("CLICKHOUSE_HTTP_PORT", "8123"))
    secure = os.environ.get("CLICKHOUSE_PROTOCOL", "http").lower() == "https"
    user = os.environ["CLICKHOUSE_USER"]
    password = os.environ["CLICKHOUSE_PASSWORD"]
    # Retry until CH answers (the Job may start before an external CH is ready).
    last_err: Exception | None = None
    for attempt in range(1, 61):
        try:
            c = clickhouse_connect.get_client(
                host=host, port=port, username=user, password=password, secure=secure
            )
            c.command("SELECT 1")
            return c
        except Exception as e:  # noqa: BLE001 — wait through any connect error
            last_err = e
            if attempt == 60:
                break
            time.sleep(2)
    raise SystemExit(f"ERROR: ClickHouse at {host}:{port} not reachable: {last_err}")


def main() -> int:
    for var in ("CLICKHOUSE_HOST", "CLICKHOUSE_USER", "CLICKHOUSE_PASSWORD", "CLICKHOUSE_DATABASE"):
        if not os.environ.get(var):
            raise SystemExit(f"ERROR: {var} must be set")

    app_db = os.environ["CLICKHOUSE_DATABASE"]
    client = _client()
    print(f"ClickHouse reachable at {os.environ['CLICKHOUSE_HOST']}", flush=True)

    print("=== dbt databases ===", flush=True)
    for db in ("staging", "silver", app_db):
        client.command(f"CREATE DATABASE IF NOT EXISTS `{db}`")

    print("=== bronze placeholders ===", flush=True)
    placeholder_stmts = _extract_heredoc_sql(PLACEHOLDERS_SH.read_text(encoding="utf-8"))
    for stmt in placeholder_stmts:
        client.command(stmt)
    print(f"  applied {len(placeholder_stmts)} placeholder statements", flush=True)

    print("=== ClickHouse migrations ===", flush=True)
    files = sorted(MIGRATIONS_DIR.glob("*.sql"))
    if not files:
        raise SystemExit(f"ERROR: no migration files under {MIGRATIONS_DIR}")
    total = 0
    for f in files:
        stmts = _split_statements(f.read_text(encoding="utf-8"))
        for stmt in stmts:
            client.command(stmt)
        total += len(stmts)
        print(f"  {f.name} ({len(stmts)} stmts)", flush=True)
    print(f"=== applied {total} statements from {len(files)} migration files ===", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
