"""Give an instance the warehouse schema a connector's first sync would have left.

A stand raised with `test-stand minimal` has identity and nothing else, so the
databases a spec seeds do not exist yet. This builds them in the order
`bootstrap-db.sh` converges a fresh cluster in:

    1. CREATE DATABASE for every database a deployment holds
    2. Run the real connectors into the real destination (`bronze.create_bronze`)
    3. Run scripts/migrations/*.sql

Bronze is created by the destination so that the rig has the same single creator a
deployment has: a table a spec seeds is the shape this tree's connectors and the
pinned destination produce. Nothing pre-creates staging, silver or gold — a spec
runs dbt for the models it asserts over, which is what a deployment's sync does.

Idempotent: every statement uses CREATE OR REPLACE / IF NOT EXISTS / DROP IF
EXISTS. We split multi-statement files on `;` because clickhouse-connect's
HTTP endpoint accepts only one statement per request.
"""

from __future__ import annotations

import logging
import os
import re
import subprocess
from pathlib import Path

from insight_datapath import clickhouse as ch
from insight_datapath.bronze import create_bronze
from insight_datapath.instance import InstanceConfig
from insight_datapath.process import tail

LOG = logging.getLogger("datapath.schema")

#: The databases a deployment gets from `src/ingestion/scripts/create-databases.sh`,
#: minus the app database, which the instance names. Nothing else creates them:
#: neither a migration nor a dbt hook carries a CREATE DATABASE.
DATABASES = (
    "staging",
    "silver",
    "identity",
    "config",
    "presentation",
    "product_usage",
    "ingestion_history",
    "insight_datasets",
)


def apply_all(cfg: InstanceConfig, *, repo_root: Path, project: str) -> int:
    """Bootstrap the warehouse, then apply every *.sql migration."""
    # 1. Every database, as create-databases.sh makes them on a deployment
    ch.ensure_database(cfg, cfg.ch_database)
    for database in DATABASES:
        ch.ensure_database(cfg, database)
    # 2. Bronze, from the connectors themselves
    create_bronze(cfg, repo_root=repo_root, project=project)

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
