"""Post-dbt ClickHouse view maintenance for the seed-once rig.

The base bootstrap (core DBs + bronze placeholders + every gold-view migration)
now runs BEFORE pytest, via the `e2e-migrate` compose service which invokes the
real src/ingestion/scripts/apply-ch-migrations.sh (see compose/docker-compose.e2e.yml).
This module only handles the two steps the rig still owns AFTER dbt materialises
the real silver:

  * reapply_migrations    — re-run scripts/migrations/*.sql so the gold views
                            rebind to the now-real silver schema.
  * refresh_intermediates — synchronously refresh every refreshable MV.

We split multi-statement files on `;` because clickhouse-connect's HTTP endpoint
accepts only one statement per request. Migrations are idempotent (CREATE OR
REPLACE / IF NOT EXISTS), so reapply is safe to repeat.
"""

from __future__ import annotations

import logging
import re
from pathlib import Path

from lib import clickhouse as ch
from lib.config import SessionConfig

LOG = logging.getLogger("e2e.migration")


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


def refresh_intermediates(cfg: SessionConfig) -> int:
    """Synchronously refresh every refreshable MV downstream of silver.

    Called once by the seed-once world build AFTER dbt materialises silver and
    the gold views are rebound. For each MV we issue `SYSTEM REFRESH VIEW` (trigger
    an immediate refresh) then `SYSTEM WAIT VIEW` (block until that refresh
    completes). WAIT (CH 24.10+) is race-free — unlike polling `system.view_refreshes`,
    which reworked its columns between 24.8 and 25.x (`last_refresh_result` is gone;
    success is now `exception = ''` + an advanced `last_success_time`). The pinned
    server is the gitops SSOT (CH 25.7 — see /docker-compose.yml).
    """
    views = discover_refreshable_views(cfg)
    if not views:
        return 0

    for view in views:
        LOG.debug("SYSTEM REFRESH VIEW %s", view)
        ch.execute(cfg, f"SYSTEM REFRESH VIEW {view}")
    for view in views:
        ch.execute(cfg, f"SYSTEM WAIT VIEW {view}")  # blocks until the refresh finishes

    # WAIT returns even for some failure modes; the `exception` column is the
    # source of truth for whether a refresh actually succeeded.
    in_list = ", ".join(f"'{v}'" for v in views)
    failed = ch.query(
        cfg,
        "SELECT concat(database, '.', view), exception FROM system.view_refreshes "
        f"WHERE concat(database, '.', view) IN ({in_list}) AND exception != ''",
    )
    if failed:
        raise RuntimeError(
            "refreshable MV(s) finished with an error:\n"
            + "\n".join(f"  {v}: {exc}" for v, exc in failed)
        )
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
