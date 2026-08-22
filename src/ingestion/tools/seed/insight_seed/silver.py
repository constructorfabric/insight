"""
ClickHouse silver-layer sample-data generation.

Table + gold-layer setup uses the SAME mechanism as a real deployment —
the seed does not reimplement any DDL. It runs the exact two scripts the
k8s clickhouse-migrate Hook Job runs, from the ingestion tree bind-mounted
at /ingestion (docker-compose.yml `seed-sample.volumes`):

1. `create-bronze-placeholders.sh` — applies the CI-generated DDL snapshot
   from scripts/connectors-ddl/*.sql (CREATE DATABASE + every bronze/silver/
   insight relation, all IF NOT EXISTS / OR REPLACE). This gives the
   generators the real production schemas to write into.

2. Generate per-team activity rows via `generators/*.py` INTO those silver
   tables. Volumes scale by team profile + persona; per-day caps live in
   each generator module.

3. `apply-ch-migrations.sh` — applies migrations/*.sql (identity DDL, class
   contract heals, the contract-version stamp), the staging label repair,
   and `dbt run --select tag:gold` to build the dbt-owned gold models. Run
   AFTER seeding so the materialized gold models are built over real seeded
   silver instead of empty placeholders.

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

from . import config
from .generators import ai, ai_cost, collab, crm, git, hr, people, support, task, wiki
from .generators.base import seed_days
from .profiles import build_seeded_roster, get_dev_user_email

LOG = logging.getLogger("seed.silver")


def _ingestion_scripts_dir() -> Path:
    """Locate src/ingestion/scripts — no env knobs needed.

    In the seed-sample container the whole ingestion tree is bind-mounted
    at /ingestion (docker-compose.yml `seed-sample.volumes`), mirroring the
    toolbox image layout the scripts resolve their relative paths against
    (apply-ch-migrations.sh cd's into ../dbt). Host runs resolve it relative
    to this file: this package lives inside the ingestion tree it seeds.
    """
    mounted = Path("/ingestion/scripts")
    if mounted.is_dir():
        return mounted
    # parents[3] = src/ingestion (silver -> insight_seed -> seed -> tools).
    return Path(__file__).resolve().parents[3] / "scripts"


def _script_env() -> dict[str, str]:
    """Env for the ingestion shell scripts (create-bronze-placeholders.sh,
    apply-ch-migrations.sh) — CLICKHOUSE_URL/USER/PASSWORD/DATABASE per
    lib/ch-exec.sh + apply-ch-migrations.sh's own asserts."""
    target = config.parse_clickhouse(os.environ)
    return {
        **os.environ,
        "CLICKHOUSE_URL": target.url,
        "CLICKHOUSE_USER": target.user,
        "CLICKHOUSE_PASSWORD": target.password,
        "CLICKHOUSE_DATABASE": target.database,
    }


def _ch_client() -> clickhouse_connect.driver.client.Client:
    target = config.parse_clickhouse(os.environ)
    # Views (gold) are created by apply-ch-migrations.sh, not this client;
    # the compose CH ships join_use_nulls=1 as a profile default
    # (deploy/compose/clickhouse-user-defaults.xml) so those CREATE VIEWs
    # type-check server-side. This client only INSERTs silver rows.
    return clickhouse_connect.get_client(
        host=target.host,
        port=target.http_port,
        username=target.user,
        password=target.password,
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
            "this package must sit inside the ingestion tree (src/ingestion/tools/seed)."
        )
    subprocess.run(["bash", str(script)], env=_script_env(), check=True)
    LOG.info("placeholders: %s applied", script.name)


# The persons-seed's input, selected by the UNION model's name: the
# silver:identity_inputs tag marks only its staging feeders.
IDENTITY_INPUTS_SELECT = "+identity_inputs"

# By model name, never by the connector tag: a tag materialises sibling
# staging models too, and the placeholder-drop hook then empties their class.
AI_INVOICE_SELECT = "claude_team__ai_invoice+"


def apply_ch_migrations(dbt_select: str | None = None, *, full_refresh: bool = False) -> None:
    """Apply gold-view migrations + build dbt-owned gold models.

    Runs the ingestion repo's apply-ch-migrations.sh — the exact script the
    k8s clickhouse-migrate Hook Job runs. It re-creates placeholders (no-op
    here), applies migrations/*.sql, repairs staging labels, and runs
    `dbt run --select tag:gold` (widened by `dbt_select` when given). Must
    run AFTER seeding so the materialized gold model reflects seeded silver
    (see module docstring).

    `full_refresh` rebuilds the selected models from source rather than
    appending to them. It is a GLOBAL dbt flag, so it reaches every selected
    model that is incremental — every `*__identity_inputs` feeder, not only
    the two named here. The silver step needs it whenever the roster can
    differ from the one already on the stand: a seed REPLACES the org, but
    the identity feeders are incremental (bamboohr__employees_snapshot
    appends; identity_inputs admits only rows past the current max _version),
    so people who are new to this stand never cross that boundary. The
    failure is silent — bronze holds the new roster, identity_inputs keeps
    describing the previous accounts, persons-seed resolves that stale set,
    and gold serves the old org with no error at any layer.
    """
    script = _ingestion_scripts_dir() / "apply-ch-migrations.sh"
    if not script.is_file():
        raise FileNotFoundError(
            f"migrations script not found at {script}. In compose, the "
            "seed-sample container must mount /ingestion; on a host run, "
            "this package must sit inside the ingestion tree (src/ingestion/tools/seed)."
        )

    env = _script_env()
    # Never inherited: the selector is exactly what the caller passes, or the
    # script's own tag:gold default.
    if dbt_select is None:
        env.pop("DBT_GOLD_SELECT", None)
    else:
        env["DBT_GOLD_SELECT"] = dbt_select
    # INVARIANT: a flag, never DBT_FULL_REFRESH — that name is
    # reconcile-connectors' and env reaches every child.
    argv = ["bash", str(script)]
    if full_refresh:
        argv.append("--full-refresh")
    subprocess.run(argv, env=env, check=True)
    LOG.info("migrations + gold: %s applied", script.name)


def generate_rows(
    client: clickhouse_connect.driver.client.Client,
) -> None:
    """Populate silver tables with per-team activity for the demo roster."""
    tenant_uuid = config.parse_tenant_id(os.environ)
    dev_email = get_dev_user_email()
    roster = build_seeded_roster(dev_email, config.parse_org_headcount(os.environ))
    # The generators' own reader, not a second copy of it: they date every row
    # from this window, and a `SEED_DAYS` the two disagreed on would put rows
    # outside the range this function logs. It also treats an empty value as
    # "unset", which a rendered Job manifest passes for a window nobody pinned.
    days = seed_days()
    LOG.info(
        "generating silver rows: tenant=%s days=%d persons=%d",
        tenant_uuid,
        days,
        len(roster),
    )

    totals: dict[str, int] = {}
    totals.update(people.generate(client, roster, tenant_uuid))
    totals.update(git.generate(client, roster, tenant_uuid, days))
    totals.update(crm.generate(client, roster, tenant_uuid, days))
    totals.update(collab.generate(client, roster, tenant_uuid, days))
    totals.update(hr.generate(client, roster, tenant_uuid, days))
    totals.update(ai.generate(client, roster, tenant_uuid, days))
    totals.update(ai_cost.generate(client, roster, tenant_uuid, days))
    totals.update(task.generate(client, roster, tenant_uuid, days))
    totals.update(support.generate(client, roster, tenant_uuid, days))
    totals.update(wiki.generate(client, roster, tenant_uuid, days))

    for table, n in sorted(totals.items()):
        LOG.info("  %-46s %6d rows", table, n)
    LOG.info("silver rows: %d total across %d tables", sum(totals.values()), len(totals))


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
        # 3. Real deploy mechanism: migrations + gold + identity inputs. Gold
        #    builds unresolved here — the orchestrator runs the persons-seed/
        #    sync pair and then the `gold` subcommand to rebuild it.
        apply_ch_migrations(
            dbt_select=f"tag:gold {IDENTITY_INPUTS_SELECT} {AI_INVOICE_SELECT}",
            full_refresh=True,
        )
    finally:
        client.close()
    LOG.info("DONE: silver rows seeded + gold layer built via deploy scripts.")


def run_gold() -> None:
    """Rebuild the dbt gold models only — gold resolves entity ids at BUILD
    time, so bindings from a persons-seed/sync land only on a rebuild."""
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
    )
    apply_ch_migrations()
    LOG.info("DONE: gold models rebuilt over the current identity map.")


if __name__ == "__main__":
    run()
