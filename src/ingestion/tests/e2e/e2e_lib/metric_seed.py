"""Seed test-specific metric definitions into MariaDB.

Runs AFTER the analytics-api binary's SeaORM auto-migrations populate prod
metrics. Reads `seed/metrics.yaml` and upserts each entry into the `metrics`
table. Idempotent — re-runs replace existing rows by id.

The fixture authors who need their own metric (e.g. a narrow SELECT used by
one test) add an entry here; broader fixtures should reference the prod-
seeded UUIDs from m20260422_000001_seed_metrics.rs.
"""

from __future__ import annotations

import logging
import uuid
from pathlib import Path

import yaml

from e2e_lib import mariadb
from e2e_lib.config import SessionConfig

LOG = logging.getLogger("e2e.metric_seed")

# All test metrics live under the nil tenant — matches the auth-stub
# context in analytics-api/src/auth.rs.
# SeaORM stores `.uuid()` columns as BINARY(16) in MariaDB, so we pass raw
# bytes — pymysql interprets a str as utf-8 (36 chars) and overflows.
NIL_TENANT = uuid.UUID("00000000-0000-0000-0000-000000000000").bytes


def seed_test_metrics(cfg: SessionConfig, seed_path: Path | None = None) -> int:
    """Read seed/metrics.yaml and upsert into MariaDB.metrics. Returns row count."""
    seed_path = seed_path or (cfg.repo_root / "src/ingestion/tests/e2e/seed/metrics.yaml")
    if not seed_path.is_file():
        LOG.debug("no seed file at %s — skipping", seed_path)
        return 0

    raw = yaml.safe_load(seed_path.read_text(encoding="utf-8"))
    overrides = (raw or {}).get("overrides") or []
    if not overrides:
        LOG.debug("seed file %s has no overrides — skipping", seed_path)
        return 0

    with mariadb.connection(cfg) as conn:
        with conn.cursor() as cur:
            for row in overrides:
                _upsert_metric(cur, row)
    LOG.info("upserted %d test metric(s) from %s", len(overrides), seed_path.name)
    return len(overrides)


def _upsert_metric(cur, row: dict) -> None:
    required = {"id", "name", "query_ref"}
    missing = required - row.keys()
    if missing:
        raise ValueError(f"seed metric missing keys {sorted(missing)}: {row!r}")

    metric_id_bytes = uuid.UUID(row["id"]).bytes
    cur.execute(
        """
        INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled)
        VALUES (%s, %s, %s, %s, %s, %s)
        ON DUPLICATE KEY UPDATE
            name = VALUES(name),
            description = VALUES(description),
            query_ref = VALUES(query_ref),
            is_enabled = VALUES(is_enabled),
            updated_at = CURRENT_TIMESTAMP
        """,
        (
            metric_id_bytes,
            NIL_TENANT,
            row["name"],
            row.get("description", ""),
            row["query_ref"],
            bool(row.get("is_enabled", True)),
        ),
    )
