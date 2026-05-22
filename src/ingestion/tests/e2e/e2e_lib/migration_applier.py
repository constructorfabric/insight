"""Apply ClickHouse migrations from src/ingestion/scripts/migrations/*.sql.

Migrations CREATE VIEW objects that reference bronze_*, silver, and staging
databases. ClickHouse 24.x validates these references at CREATE-time, so we
must materialize the bronze placeholder schemas BEFORE running migrations —
mirroring the prod order from src/ingestion/scripts/init.sh:

    1. CREATE DATABASE staging | silver | insight
    2. Run src/ingestion/scripts/create-bronze-placeholders.sh
    3. Run scripts/migrations/*.sql

This module does (1)+(2)+(3) in the test ClickHouse. We parse the bash
heredocs out of `create-bronze-placeholders.sh` rather than duplicating the
DDL — keeps the test rig in lock-step with prod schema evolution.

Idempotent: every statement uses CREATE OR REPLACE / IF NOT EXISTS / DROP IF
EXISTS. We split multi-statement files on `;` because clickhouse-connect's
HTTP endpoint accepts only one statement per request.
"""

from __future__ import annotations

import logging
import re
from pathlib import Path

from e2e_lib import clickhouse as ch
from e2e_lib.config import SessionConfig

LOG = logging.getLogger("e2e.migration")


def apply_all(cfg: SessionConfig) -> int:
    """Bootstrap databases + placeholders, then apply every *.sql migration."""
    # 1. App DB exists (some migrations DROP VIEW insight.* before recreating).
    ch.ensure_database(cfg, cfg.ch_database)
    # 2. staging DB — dbt models live here in prod
    ch.ensure_database(cfg, "staging")
    # 3. Bronze placeholders (creates silver DB + all class_* placeholder tables)
    bronze_count = apply_bronze_placeholders(cfg)
    LOG.info("applied %d bronze-placeholder statements", bronze_count)

    files = sorted(cfg.migrations_dir.glob("*.sql"))
    if not files:
        raise RuntimeError(f"no migration files found under {cfg.migrations_dir}")

    total = 0
    for f in files:
        LOG.info("applying migration: %s", f.name)
        total += _apply_file(cfg, f)
    LOG.info("applied %d statements from %d migration files", total, len(files))
    return total


def apply_bronze_placeholders(cfg: SessionConfig) -> int:
    """Parse `create-bronze-placeholders.sh` heredocs and run the SQL.

    The prod script invokes `kubectl exec` to talk to the in-cluster CH; we
    extract just the SQL between `run_ch <<'SQL'` ... `SQL` markers and run
    it via our HTTP client.
    """
    script = cfg.repo_root / "src/ingestion/scripts/create-bronze-placeholders.sh"
    if not script.exists():
        raise RuntimeError(f"placeholder script missing: {script}")

    statements = _extract_heredoc_sql(script.read_text(encoding="utf-8"))
    for stmt in statements:
        ch.execute(cfg, stmt)
    return len(statements)


def discover_refreshable_views(cfg: SessionConfig) -> list[str]:
    """Auto-discover every refreshable MV via `system.view_refreshes`.

    Source of truth = ClickHouse itself, not a hardcoded list. When prod
    migrations add a new refreshable MV, the rig picks it up automatically
    on the next test run — no edits to the framework needed.
    """
    rows = ch.query(
        cfg,
        "SELECT concat(database, '.', view) FROM system.view_refreshes ORDER BY database, view",
    )
    return [r[0] for r in rows]


def refresh_intermediates(cfg: SessionConfig, *, timeout_s: float = 30.0) -> int:
    """Trigger a synchronous refresh of every refreshable MV downstream of silver.

    Called by the per-test fixture AFTER seeding silver and BEFORE calling the
    API. `SYSTEM REFRESH VIEW` is fire-and-forget in CH 24.8 — we poll
    `system.view_refreshes` until each MV's status is `Finished`.
    `SYSTEM WAIT VIEW` only landed in CH 24.10+; this implementation is
    compatible with 24.8 (prod version pinned in compose/docker-compose.yml).
    """
    import time

    views = discover_refreshable_views(cfg)
    if not views:
        return 0

    for view in views:
        LOG.debug("SYSTEM REFRESH VIEW %s", view)
        ch.execute(cfg, f"SYSTEM REFRESH VIEW {view}")

    in_list = ", ".join(f"'{v}'" for v in views)
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        rows = ch.query(
            cfg,
            "SELECT concat(database, '.', view), status, last_refresh_result "
            f"FROM system.view_refreshes WHERE concat(database, '.', view) IN ({in_list})",
        )
        all_done = (
            len(rows) == len(views)
            and all(
                (status == "Scheduled" and result == "Finished") or status == "Finished"
                for (_v, status, result) in rows
            )
        )
        if all_done:
            LOG.info("refreshed %d intermediate views: %s", len(views), views)
            return len(views)
        time.sleep(0.2)

    # Final state for diagnostics
    rows = ch.query(
        cfg,
        "SELECT concat(database, '.', view), status, last_refresh_result, exception "
        "FROM system.view_refreshes",
    )
    raise RuntimeError(
        f"refresh of intermediates timed out after {timeout_s}s; current state:\n"
        + "\n".join(f"  {r}" for r in rows)
    )


def _extract_heredoc_sql(bash_source: str) -> list[str]:
    """Pull the body of every `run_ch <<'SQL' ... SQL` heredoc, then split on `;`."""
    parts: list[str] = []
    in_block = False
    buf: list[str] = []
    for line in bash_source.splitlines():
        if not in_block and re.match(r"^\s*run_ch\s+<<'SQL'\s*$", line):
            in_block = True
            buf = []
            continue
        if in_block and re.match(r"^SQL\s*$", line):
            in_block = False
            parts.append("\n".join(buf))
            continue
        if in_block:
            buf.append(line)
    # Each part may contain multiple statements separated by `;`
    statements: list[str] = []
    for part in parts:
        for stmt in _split_statements(part):
            if stmt:
                statements.append(stmt)
    return statements


def _apply_file(cfg: SessionConfig, path: Path) -> int:
    sql = path.read_text(encoding="utf-8")
    statements = _split_statements(sql)
    for stmt in statements:
        if not stmt.strip():
            continue
        ch.execute(cfg, stmt)
    return len(statements)


_COMMENT_LINE = re.compile(r"^\s*--.*$", re.MULTILINE)


def _split_statements(sql: str) -> list[str]:
    """Strip SQL line-comments and split on `;`.

    ClickHouse migration files in this repo do not use string literals containing
    `;` or stored procedures, so a naive split is safe. If that ever changes, we
    rewrite this on top of a real tokenizer.
    """
    stripped = _COMMENT_LINE.sub("", sql)
    parts = [p.strip() for p in stripped.split(";")]
    return [p for p in parts if p]
