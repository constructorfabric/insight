"""Apply ClickHouse migrations from src/ingestion/scripts/migrations/*.sql.

Migrations CREATE VIEW objects that reference bronze_*, silver, and staging
databases. ClickHouse 24.x validates these references at CREATE-time, so we
must materialize the bronze/silver schemas BEFORE running migrations —
mirroring the prod order from src/ingestion/scripts/apply-ch-migrations.sh:

    1. CREATE DATABASE staging | silver | insight
    2. Apply the scripts/connectors-ddl/*.sql snapshot
       (what create-bronze-placeholders.sh does in prod)
    3. Run scripts/migrations/*.sql

The connectors-ddl snapshot is CI-generated from the real connectors and dbt
models (see .github/workflows/connectors-ddl.yml), so the test rig stays in
lock-step with prod schema evolution by construction.

Idempotent: every statement uses CREATE OR REPLACE / IF NOT EXISTS / DROP IF
EXISTS. We split multi-statement files on `;` because clickhouse-connect's
HTTP endpoint accepts only one statement per request.
"""

from __future__ import annotations

import logging
import re
from pathlib import Path

from lib import clickhouse as ch
from lib.config import SessionConfig

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


def reapply_migrations(cfg: SessionConfig) -> int:
    """Re-run only the *.sql migrations (no placeholder bootstrap).

    Gold views are CREATE-d at session start against the reduced silver
    PLACEHOLDER schema. Once a fixture's `dbt build` materialises the real
    silver schema (different nullability), a view's frozen result structure no
    longer matches what it now returns, and reading it inside a date-filter
    subquery raises ClickHouse `INCORRECT_QUERY` (Nullable/`join_use_nulls`
    mismatch). On a long-lived cluster (dev/prod) the views were created against
    the real silver, so this never bites there — verified: the same query runs
    clean against dev. Re-running the migrations after dbt recreates every
    `DROP VIEW IF EXISTS ... CREATE VIEW` against the now-real silver, realigning
    the structure. The migrations are idempotent (verified), so this is safe to
    repeat per fixture.
    """
    files = sorted(cfg.migrations_dir.glob("*.sql"))
    if not files:
        raise RuntimeError(f"no migration files found under {cfg.migrations_dir}")
    total = 0
    for f in files:
        total += _apply_file(cfg, f)
    LOG.info("re-applied %d statements from %d migration files (post-dbt view refresh)", total, len(files))
    return total


def apply_bronze_placeholders(cfg: SessionConfig) -> int:
    """Apply the scripts/connectors-ddl/*.sql snapshot.

    Same order and retry semantics as prod's create-bronze-placeholders.sh:
    per-connector bronze files first, then silver.sql, then insight.sql.
    Views may reference other views, so failed statements are retried in
    additional passes until a pass makes no progress.
    """
    ddl_dir = cfg.repo_root / "src/ingestion/scripts/connectors-ddl"
    files = sorted(ddl_dir.glob("*.sql"))
    if not files:
        raise RuntimeError(f"no DDL snapshot files under {ddl_dir}")

    ordered = [f for f in files if f.stem not in ("silver", "insight")] + [
        ddl_dir / "silver.sql",
        ddl_dir / "insight.sql",
    ]
    pending: list[str] = []
    for f in ordered:
        pending.extend(_split_statements(f.read_text(encoding="utf-8")))

    applied = 0
    while pending:
        failed: list[tuple[str, Exception]] = []
        for stmt in pending:
            try:
                ch.execute(cfg, stmt)
                applied += 1
            except Exception as exc:  # noqa: BLE001 — retried next pass
                failed.append((stmt, exc))
        if len(failed) == len(pending):
            summary = "\n".join(f"  {s[:120]!r}: {e}" for s, e in failed[:5])
            raise RuntimeError(f"DDL snapshot stuck; {len(failed)} statement(s) keep failing:\n{summary}")
        pending = [s for s, _ in failed]
    return applied


def discover_refreshable_views(cfg: SessionConfig) -> list[str]:
    """Auto-discover every refreshable MV via `system.view_refreshes`.

    Source of truth = ClickHouse itself, not a hardcoded list. When prod
    migrations add a new refreshable MV, the rig picks it up automatically
    on the next test run — no edits to the framework needed.
    """
    rows = ch.query(cfg, "SELECT concat(database, '.', view) FROM system.view_refreshes ORDER BY database, view")
    return [r[0] for r in rows]


def refresh_intermediates(cfg: SessionConfig) -> int:
    """Trigger a synchronous refresh of every refreshable MV downstream of silver.

    Called by the per-test fixture AFTER seeding silver and BEFORE calling the
    API. `SYSTEM REFRESH VIEW` schedules the refresh and `SYSTEM WAIT VIEW`
    blocks until it completes (and raises if it failed). Verified against the
    pinned ClickHouse 25.7.5: REFRESH→WAIT→read shows no stale-read race
    (40/40 iterations); the refresh_count polling this replaces was only
    needed on 24.8, where SYSTEM WAIT VIEW did not exist yet.
    """
    views = discover_refreshable_views(cfg)
    if not views:
        return 0

    for view in views:
        LOG.debug("SYSTEM REFRESH VIEW %s", view)
        ch.execute(cfg, f"SYSTEM REFRESH VIEW {view}")
    for view in views:
        ch.execute(cfg, f"SYSTEM WAIT VIEW {view}")

    LOG.info("refreshed %d intermediate views: %s", len(views), views)
    return len(views)


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
