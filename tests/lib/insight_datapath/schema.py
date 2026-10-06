"""Give an instance the warehouse schema a connector's first sync would have left.

A stand raised with `test-stand minimal` has identity and nothing else, so the
databases a spec seeds do not exist yet. This builds them in the order
`bootstrap-db.sh` converges a fresh cluster in:

    1. CREATE DATABASE staging | insight
    2. Run the real connectors into the real destination (`bronze.create_bronze`)
    3. Apply the warehouse half of the scripts/connectors-ddl snapshot
    4. Run scripts/migrations/*.sql

Bronze is created by the destination rather than from the snapshot so that the rig
has the same single creator a deployment has: a table a spec seeds is the shape this
tree's connectors and the pinned destination produce, not the shape the snapshot
carried when it was last dumped. The snapshot's four non-connector files still stand
in for relations a deployment gets from dbt (T23 retires them).

Idempotent: every statement uses CREATE OR REPLACE / IF NOT EXISTS / DROP IF
EXISTS. We split multi-statement files on `;` because clickhouse-connect's
HTTP endpoint accepts only one statement per request.
"""

from __future__ import annotations

import importlib.util
import logging
import os
import re
import subprocess
import sys
from functools import lru_cache
from pathlib import Path
from types import ModuleType

from insight_datapath import clickhouse as ch
from insight_datapath.bronze import create_bronze
from insight_datapath.instance import InstanceConfig
from insight_datapath.process import tail

LOG = logging.getLogger("datapath.schema")

#: The snapshot files that are not a connector's bronze, applied in dependency
#: order: `insight`'s views read `silver` and every `bronze_*` database.
WAREHOUSE_SNAPSHOT = ("identity", "staging", "silver", "insight")


def apply_all(cfg: InstanceConfig, *, repo_root: Path, project: str) -> int:
    """Bootstrap the warehouse, then apply every *.sql migration."""
    # 1. App DB exists (some migrations DROP VIEW insight.* before recreating).
    ch.ensure_database(cfg, cfg.ch_database)
    # 2. staging DB — dbt models live here in prod
    ch.ensure_database(cfg, "staging")
    # 3. Bronze, from the connectors themselves
    create_bronze(cfg, repo_root=repo_root, project=project)
    # 4. identity/staging/silver/insight, which a deployment gets from dbt
    applied = apply_warehouse_snapshot(cfg, repo_root=repo_root)
    LOG.info("applied %d warehouse-snapshot statements", applied)

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


def apply_warehouse_snapshot(cfg: InstanceConfig, *, repo_root: Path) -> int:
    """Apply the non-connector half of the scripts/connectors-ddl snapshot.

    Same retry semantics as prod's create-bronze-placeholders.sh: views may
    reference other views, so failed statements are retried in additional passes
    until a pass makes no progress.
    """
    ddl_dir = repo_root / "src/ingestion/scripts/connectors-ddl"
    ordered = [ddl_dir / f"{stem}.sql" for stem in WAREHOUSE_SNAPSHOT]
    absent = [f.name for f in ordered if not f.is_file()]
    if absent:
        raise RuntimeError(f"missing DDL snapshot file(s) under {ddl_dir}: {', '.join(absent)}")

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


def restart_analytics(
    *, repo_root: Path, project: str, env_file: Path, timeout_s: float = 300.0
) -> None:
    """Make analytics re-read which metrics it can serve.

    It decides that once, at startup, from the relations present then. On an instance
    whose warehouse was empty when it booted, every metric key is reported unknown
    until it looks again.
    """
    try:
        result = subprocess.run(
            [
                "docker",
                "compose",
                "--project-name",
                project,
                "--env-file",
                str(env_file),
                "-f",
                "docker-compose.yml",
                "restart",
                "analytics",
            ],
            cwd=repo_root,
            env={**os.environ, "COMPOSE_PROJECT_NAME": project},
            capture_output=True,
            text=True,
            check=False,
            timeout=timeout_s,
        )
    except subprocess.TimeoutExpired as timeout:
        raise RuntimeError(
            f"analytics did not restart within {timeout_s:.0f}s:\n{tail(timeout.stderr)}"
        ) from timeout
    if result.returncode != 0:
        raise RuntimeError(
            f"could not restart analytics (exit {result.returncode}):\n{result.stderr[-1000:]}"
        )
    LOG.info("analytics restarted; its metric catalogue is re-read")
