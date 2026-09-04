"""Give an instance the warehouse schema a connector's first sync would have left.

A stand raised with `test-stand minimal` has identity and nothing else, so the
databases a spec seeds do not exist yet. This applies what a deployment applies, in
the deployment's order (`src/ingestion/scripts/apply-ch-migrations.sh`):

    1. CREATE DATABASE staging | silver | insight
    2. Apply the scripts/connectors-ddl/*.sql snapshot
       (what create-bronze-placeholders.sh does in prod)
    3. Run scripts/migrations/*.sql

The connectors-ddl snapshot is generated from the real connectors and dbt models and
validated on every PR, so this stays in lock-step with the schema a deployment gets.

Idempotent: every statement uses CREATE OR REPLACE / IF NOT EXISTS / DROP IF
EXISTS. We split multi-statement files on `;` because clickhouse-connect's
HTTP endpoint accepts only one statement per request.
"""

from __future__ import annotations

import importlib.util
import logging
import re
import sys
from functools import lru_cache
from pathlib import Path
from types import ModuleType

from insight_datapath import clickhouse as ch
from insight_datapath.instance import InstanceConfig

LOG = logging.getLogger("datapath.schema")


def apply_all(cfg: InstanceConfig, *, repo_root: Path) -> int:
    """Bootstrap databases + placeholders, then apply every *.sql migration."""
    # 1. App DB exists (some migrations DROP VIEW insight.* before recreating).
    ch.ensure_database(cfg, cfg.ch_database)
    # 2. staging DB — dbt models live here in prod
    ch.ensure_database(cfg, "staging")
    # 3. Bronze placeholders (creates silver DB + all class_* placeholder tables)
    bronze_count = apply_bronze_placeholders(cfg, repo_root=repo_root)
    LOG.info("applied %d bronze-placeholder statements", bronze_count)

    migrations_dir = repo_root / "src/ingestion/scripts/migrations"
    files = sorted(migrations_dir.glob("*.sql"))
    if not files:
        raise RuntimeError(f"no migration files found under {migrations_dir}")

    total = 0
    for f in files:
        LOG.info("applying migration: %s", f.name)
        total += _apply_file(cfg, f)
    LOG.info("applied %d statements from %d migration files", total, len(files))
    return total


def apply_bronze_placeholders(cfg: InstanceConfig, *, repo_root: Path) -> int:
    """Apply the scripts/connectors-ddl/*.sql snapshot.

    Same order and retry semantics as prod's create-bronze-placeholders.sh:
    per-connector bronze files first, then silver.sql, then insight.sql.
    Views may reference other views, so failed statements are retried in
    additional passes until a pass makes no progress.
    """
    ddl_dir = repo_root / "src/ingestion/scripts/connectors-ddl"
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
            except Exception as exc:
                failed.append((stmt, exc))
        if len(failed) == len(pending):
            summary = "\n".join(f"  {s[:120]!r}: {e}" for s, e in failed[:5])
            raise RuntimeError(
                f"DDL snapshot stuck; {len(failed)} statement(s) keep failing:\n{summary}"
            )
        pending = [s for s, _ in failed]

    reconcile_bronze_schema(cfg, ddl_dir, repo_root=repo_root)
    return applied


def reconcile_bronze_schema(cfg: InstanceConfig, ddl_dir: Path, *, repo_root: Path) -> int:
    """Add snapshot columns missing from pre-existing bronze tables.

    Mirrors the phase prod runs at the end of create-bronze-placeholders.sh, by
    importing the same module rather than reimplementing it — the rig's
    ClickHouse outlives a single run (compose volume, and CI reuses the service
    across fixtures), so it accumulates exactly the schema drift #1991 is about.
    """
    reconciler = _reconciler(repo_root / "src/ingestion/scripts/reconcile_bronze_schema.py")
    result = reconciler.reconcile(
        reconciler.load_snapshot_tables(ddl_dir),
        execute=lambda sql: ch.execute(cfg, sql),
        fetch_rows=lambda sql: [[str(cell) for cell in row] for row in ch.query(cfg, sql)],
    )
    if result.columns_added:
        LOG.info(
            "reconciled %d bronze column(s) across %d table(s)",
            result.columns_added,
            result.tables_reconciled,
        )
    for qualified, name, snapshot_type, live_type in result.type_drift:
        LOG.warning(
            "%s.%s type differs — snapshot=%s live=%s (left unchanged)",
            qualified,
            name,
            snapshot_type,
            live_type,
        )
    return result.columns_added


@lru_cache(maxsize=1)
def _reconciler(path: Path) -> ModuleType:
    """Load scripts/reconcile_bronze_schema.py, which lives outside the rig's package root.

    The module must be registered in sys.modules BEFORE exec_module: dataclass
    resolves its own module via `sys.modules[cls.__module__]`, so executing an
    unregistered module raises AttributeError on the first @dataclass. Loading
    the file by path (rather than putting scripts/ on sys.path) keeps the rig's
    own `tests` package from being shadowed by the one next to the script.
    """
    spec = importlib.util.spec_from_file_location("reconcile_bronze_schema", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load the bronze reconciler from {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _apply_file(cfg: InstanceConfig, path: Path) -> int:
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
