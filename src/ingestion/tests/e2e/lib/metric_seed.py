"""Seed test-specific metric definitions into MariaDB.

Runs AFTER the analytics binary's SeaORM auto-migrations populate prod
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

from lib import mariadb
from lib.config import TEST_TENANT_ID, SessionConfig

LOG = logging.getLogger("e2e.metric_seed")

# All e2e metric definitions live under TEST_TENANT_ID. The analytics tenant
# middleware rejects the nil UUID, and `find_enabled_metric` filters the
# `metrics` table by tenant, so both the prod metrics seeded by the binary's
# migrations (under the nil tenant) and our overrides must sit under the tenant
# the harness's signed gateway JWT carries in its `tenant_id` claim.
# SeaORM stores `.uuid()` columns as BINARY(16) in MariaDB, so we pass raw
# bytes — pymysql interprets a str as utf-8 (36 chars) and overflows.
TEST_TENANT = TEST_TENANT_ID.bytes
NIL_TENANT = uuid.UUID("00000000-0000-0000-0000-000000000000").bytes


def seed_test_metrics(cfg: SessionConfig, seed_path: Path | None = None) -> int:
    """Align MariaDB.metrics with the e2e tenant, then upsert seed overrides.

    Runs after the analytics binary's SeaORM migrations have seeded the prod
    metric catalog (under the nil tenant). Returns the number of override rows.
    """
    seed_path = seed_path or (cfg.repo_root / "src/ingestion/tests/e2e/seed/metrics.yaml")
    overrides: list[dict] = []
    if seed_path.is_file():
        raw = yaml.safe_load(seed_path.read_text(encoding="utf-8"))
        overrides = (raw or {}).get("overrides") or []

    with mariadb.connection(cfg) as conn:
        with conn.cursor() as cur:
            moved = _retenant_seeded_metrics(cur)
            for row in overrides:
                _upsert_metric(cur, row)
    LOG.info(
        "re-tenanted %d migration-seeded metric(s) onto %s; upserted %d override(s)",
        moved,
        TEST_TENANT_ID,
        len(overrides),
    )
    return len(overrides)


def _retenant_seeded_metrics(cur) -> int:
    """Move metrics the binary seeded under the nil tenant onto TEST_TENANT.

    The query path's `find_enabled_metric` is tenant-scoped, so prod metrics
    seeded under 0000…0 are invisible to a request that resolves to TEST_TENANT.
    Re-homing them in the test DB (NOT in the migration source) keeps the fix
    inside the harness and out of prod seeding. Idempotent."""
    cur.execute("UPDATE metrics SET insight_tenant_id = %s WHERE insight_tenant_id = %s", (TEST_TENANT, NIL_TENANT))
    return cur.rowcount


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
            TEST_TENANT,
            row["name"],
            row.get("description", ""),
            row["query_ref"],
            bool(row.get("is_enabled", True)),
        ),
    )
