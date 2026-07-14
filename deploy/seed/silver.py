"""
ClickHouse silver-layer sample-data generation.

Table + gold-layer setup uses the SAME mechanism as a real deployment —
the seed does not reimplement any DDL. It runs the exact two scripts the
k8s clickhouse-migrate Hook Job runs, from the ingestion tree bind-mounted
at /ingestion (docker-compose.yml `seed-sample.volumes`):

1. `create-bronze-placeholders.sh` — CREATE DATABASE + bronze/silver
   placeholder tables (CREATE TABLE IF NOT EXISTS; each silver placeholder
   carries the INSIGHT_PLACEHOLDER_v1 marker). This gives the generators
   real tables to write into.

2. Generate per-team activity rows via `generators/*.py` INTO those silver
   tables. Volumes scale by team profile + persona; per-day caps live in
   each generator module.

3. `apply-ch-migrations.sh` — applies migrations/*.sql (gold VIEWs), the
   staging label repair, and `dbt run --select tag:gold` to build the
   dbt-owned gold models. Run AFTER seeding so the one materialized gold
   model (`insight.git_metric_observations`, materialized='table') is built
   over real seeded silver instead of empty placeholders. The migration
   views and the two view-materialized gold models are read-time, so their
   order relative to seeding does not matter; the table model's does.

   Re-running create-bronze-placeholders.sh from inside this script is a
   no-op on the seeded tables (IF NOT EXISTS, no DROP/TRUNCATE), and the
   dbt on-run-start `drop_silver_placeholders_at_start` hook does NOT fire
   because `--select tag:gold` never materializes a staging model (the
   hook's required second factor) — so the seeded rows survive.

All steps are idempotent — re-running converges on the same end state.
"""

from __future__ import annotations

import logging
import os
import subprocess
from pathlib import Path

import clickhouse_connect

from generators import ai, collab, crm, git, hr, people, support, task
from profiles import build_roster, get_dev_user_email

LOG = logging.getLogger("seed.silver")

DEFAULT_DAYS = 60


def _ingestion_scripts_dir() -> Path:
    """Locate src/ingestion/scripts — no env knobs needed.

    In the seed-sample container the whole ingestion tree is bind-mounted
    at /ingestion (docker-compose.yml `seed-sample.volumes`), mirroring the
    toolbox image layout the scripts resolve their relative paths against
    (apply-ch-migrations.sh cd's into ../dbt). Host runs resolve it relative
    to this file: deploy/seed lives in the same repo as src/ingestion, two
    levels below the root.
    """
    mounted = Path("/ingestion/scripts")
    if mounted.is_dir():
        return mounted
    # parents[2] = repo root (silver.py -> seed -> deploy -> root).
    return Path(__file__).resolve().parents[2] / "src/ingestion/scripts"


def _script_env() -> dict[str, str]:
    """Env for the ingestion shell scripts (create-bronze-placeholders.sh,
    apply-ch-migrations.sh) — CLICKHOUSE_URL/USER/PASSWORD/DATABASE per
    lib/ch-exec.sh + apply-ch-migrations.sh's own asserts."""
    host = os.environ.get("CLICKHOUSE_HOST", "clickhouse")
    port = os.environ.get("CLICKHOUSE_HTTP_PORT", "8123")
    return {
        **os.environ,
        "CLICKHOUSE_URL": f"http://{host}:{port}",
        "CLICKHOUSE_USER": os.environ.get("CLICKHOUSE_USER", "insight"),
        "CLICKHOUSE_PASSWORD": os.environ.get("CLICKHOUSE_PASSWORD", "insight-local"),
        "CLICKHOUSE_DATABASE": os.environ.get("CLICKHOUSE_DATABASE", "insight"),
    }


def _ch_client() -> clickhouse_connect.driver.client.Client:
    host = os.environ.get("CLICKHOUSE_HOST", "clickhouse")
    port = int(os.environ.get("CLICKHOUSE_HTTP_PORT", "8123"))
    user = os.environ.get("CLICKHOUSE_USER", "insight")
    pwd = os.environ.get("CLICKHOUSE_PASSWORD", "insight-local")
    # Views (gold) are created by apply-ch-migrations.sh, not this client;
    # the compose CH ships join_use_nulls=1 as a profile default
    # (deploy/compose/clickhouse-user-defaults.xml) so those CREATE VIEWs
    # type-check server-side. This client only INSERTs silver rows.
    return clickhouse_connect.get_client(
        host=host, port=port, username=user, password=pwd,
    )


def apply_create_bronze_placeholders() -> None:
    """CREATE DATABASE + bronze/silver placeholder tables.

    Runs the ingestion repo's create-bronze-placeholders.sh — the exact
    script the k8s clickhouse-migrate Hook Job runs — so placeholder DDL
    has a single source of truth and cannot drift.
    """
    script = _ingestion_scripts_dir() / "create-bronze-placeholders.sh"
    if not script.is_file():
        raise FileNotFoundError(
            f"placeholders script not found at {script}. In compose, the "
            "seed-sample container must mount /ingestion; on a host run, "
            "deploy/seed must sit inside the insight repo next to src/ingestion."
        )
    subprocess.run(["bash", str(script)], env=_script_env(), check=True)
    LOG.info("placeholders: %s applied", script.name)


def apply_ch_migrations() -> None:
    """Apply gold-view migrations + build dbt-owned gold models.

    Runs the ingestion repo's apply-ch-migrations.sh — the exact script the
    k8s clickhouse-migrate Hook Job runs. It re-creates placeholders (no-op
    here), applies migrations/*.sql, repairs staging labels, and runs
    `dbt run --select tag:gold`. Must run AFTER seeding so the materialized
    gold model reflects seeded silver (see module docstring).
    """
    script = _ingestion_scripts_dir() / "apply-ch-migrations.sh"
    if not script.is_file():
        raise FileNotFoundError(
            f"migrations script not found at {script}. In compose, the "
            "seed-sample container must mount /ingestion; on a host run, "
            "deploy/seed must sit inside the insight repo next to src/ingestion."
        )
    subprocess.run(["bash", str(script)], env=_script_env(), check=True)
    LOG.info("migrations + gold: %s applied", script.name)


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
    totals.update(people.generate(client, roster, tenant_uuid))
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
    # 1. Real deploy mechanism: create the placeholder tables.
    apply_create_bronze_placeholders()
    client = _ch_client()
    try:
        LOG.info("ClickHouse version: %s", client.server_version)
        # 2. Seed silver rows into the created tables.
        generate_rows(client)
        # 3. Real deploy mechanism: migrations + gold (incl. dbt gold build
        #    over the now-seeded silver). Runs AFTER seeding so the one
        #    materialized gold model (git_metric_observations, a table) and
        #    the migration views all reflect real rows.
        apply_ch_migrations()
        # 4. Populate the task refreshable MVs from seeded silver — DDL was
        #    created in step 3; SYSTEM REFRESH must run once the rows exist.
        #    Python analog of scripts/post-deploy/refresh-task-views.sh (that
        #    script needs clickhouse-client, which the seed image omits to
        #    stay lean — the two SYSTEM REFRESH statements are identical).
        task.refresh_dependent_mvs(client)
        LOG.info("task refreshable MVs refreshed")
    finally:
        client.close()
    LOG.info("DONE: silver rows seeded + gold layer built via deploy scripts.")


if __name__ == "__main__":
    run()
