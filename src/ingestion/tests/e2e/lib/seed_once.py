"""Seed-once world builder: populate the whole stack from ALL fixtures at once.

Called once per session (see conftest.build_world fixture). Every fixture's
bronze rows are namespaced (lib.namespace) so they can coexist without
ReplacingMergeTree collapse or cross-fixture aggregate/join bleed, then seeded
together:

    seed all namespaced bronze
      -> dbt build staging (union of every fixture's touched models)
      -> connector enrich (once, over all seeded source_ids)
      -> dbt build silver (union + ephemeral enrich-fed targets)
      -> reapply gold-view migrations ONCE  (realign views to real silver)
      -> refresh refreshable MVs ONCE

The `e2e-migrate` compose service already applied all migrations (so
`identity`/`person` exist for dbt to write into) with gold views bound to the
silver placeholders. dbt then drops those placeholders and materialises real
silver, so the views must be rebound once — that is the single
`reapply_migrations` below. It replaces the old per-fixture reapply (≈40 views ×
every fixture → once per session); the subsequent per-test path does no DDL at all.
"""

from __future__ import annotations

import logging

from lib import clickhouse as ch
from lib import namespace
from lib.ch_seeder import CHSeeder
from lib.dbt_runner import DbtRunner
from lib.enrich import EnrichRunner
from lib.fixture_loader import TestYaml
from lib.migration_applier import reapply_migrations as apply_gold_migrations
from lib.migration_applier import refresh_intermediates
from lib.worker import WorkerContext

LOG = logging.getLogger("e2e.seed_once")


# Silver/staging tables that a fixture may READ via a gold view but NOT seed
# (each collab fixture seeds at most one class_collab_* table, yet
# insight.collab_bullet_rows reads all four — and each class_collab_* unions
# several per-source staging feeders). The seed step only truncates the bronze it
# seeds, so on a WARM ClickHouse (re-running `./e2e.sh test` without `down`) the
# first collab fixture would inherit a prior session's rows in the tables it does
# not seed — stale rows in a dbt-rebuilt class_collab_* would skew its neighbours.
# The zoom staging models are also `incremental`/`append`, so a warm rebuild would
# ALSO accumulate duplicate unique_keys (failing their dbt `unique` test).
# Truncating these once at build start makes warm re-runs deterministic; CI starts
# fresh anyway. (The e2e-migrate step created these tables; here we just clear them.)
_SESSION_START_TRUNCATE = [
    ("silver", "class_collab_email_activity"),
    ("silver", "class_collab_chat_activity"),
    ("silver", "class_collab_meeting_activity"),
    ("silver", "class_collab_document_activity"),
    ("staging", "m365__collab_email_activity"),
    ("staging", "m365__collab_chat_activity"),
    ("staging", "m365__collab_meeting_activity"),
    ("staging", "m365__collab_document_activity_onedrive"),
    ("staging", "m365__collab_document_activity_sharepoint"),
    # Zoom feeds class_collab_meeting_activity (cross-source meeting_hours).
    ("staging", "zoom__collab_meeting_activity"),
    ("staging", "zoom__meeting_sessions"),
    # Task-tracking: the bullet/MV chain reads class_task_* even when a fixture
    # seeds only one connector, and the enrich path writes staging.jira__task_*.
    ("silver", "class_task_field_history"),
    ("silver", "class_task_users"),
    ("silver", "class_task_field_metadata"),
    ("silver", "class_task_worklogs"),
    ("staging", "jira__task_field_history"),
    ("staging", "jira_issue_field_snapshot"),
    ("staging", "jira_changelog_items"),
    ("staging", "jira__task_field_metadata"),
    # claude_team / claude_enterprise / chatgpt_team / cursor build incremental
    # `append` staging models with a dbt `unique` test on unique_key — a warm
    # re-run (reused CH volume) would accumulate duplicate keys without a reset.
    ("staging", "claude_team__ai_dev_usage"),
    ("staging", "claude_team__ai_overage"),
    ("staging", "claude_enterprise__ai_dev_usage"),
    ("staging", "cursor__ai_dev_usage"),
    ("staging", "chatgpt_team__ai_dev_usage"),
    ("staging", "chatgpt_team__ai_assistant_usage"),
    # Wiki: class_wiki_* are incremental (delete+insert, `_version > max`) and union
    # BOTH outline + confluence. A warm re-run with the same seed _version produces
    # no new rows, leaving the prior test's data in place. Reset so max(_version)=0
    # and the first test's real millis _version reloads fully.
    ("silver", "class_wiki_pages"),
    ("silver", "class_wiki_engagement"),
    ("silver", "class_wiki_activity"),
]


def _reset_multi_reader_tables(seeder: CHSeeder) -> None:
    """Truncate the shared incremental/multi-reader tables for warm-rerun
    determinism (see _SESSION_START_TRUNCATE). No-op-safe: IF EXISTS."""
    for schema, table in _SESSION_START_TRUNCATE:
        ch.execute(seeder.cfg, f"TRUNCATE TABLE IF EXISTS `{schema}`.`{table}`")


def merge_namespaced_bronze(fixtures: list[TestYaml]) -> dict[str, list[dict]]:
    """Union every fixture's bronze, each record namespaced by its fixture token.

    Returns a single `table_fqn -> [records]` map ready for one `seed_bronze`.
    """
    merged: dict[str, list[dict]] = {}
    for ty in fixtures:
        token = namespace.token_for(ty.name)
        for tbl, rows in namespace.namespace_bronze(ty.bronze, token).items():
            merged.setdefault(tbl, []).extend(rows)
    return merged


def build_world(
    *,
    seeder: CHSeeder,
    dbt_runner: DbtRunner,
    enrich_runner: EnrichRunner,
    fixtures: list[TestYaml],
    worker_ctx: WorkerContext,
) -> None:
    """Seed all fixtures and build the stack once, in prod order.

    ClickHouse is already migrated (docker compose + the e2e-migrate service ran
    the real apply-ch-migrations.sh: core DBs, bronze placeholders, gold views).
    This starts from that migrated-but-empty stack.
    """
    merged = merge_namespaced_bronze(fixtures)
    if not merged:
        LOG.warning("no bronze across %d fixtures — nothing to seed", len(fixtures))
        return

    total_rows = sum(len(r) for r in merged.values())
    LOG.info(
        "seed-once: %d fixtures, %d bronze tables, %d rows",
        len(fixtures), len(merged), total_rows,
    )

    # 0. Warm-rerun determinism: clear the shared incremental/multi-reader tables.
    _reset_multi_reader_tables(seeder)

    # 1. Seed the merged bronze (seed_bronze truncates each table then inserts).
    seeder.seed_bronze(merged)
    touched = {(fqn.split(".", 1)[0], fqn.split(".", 1)[1]) for fqn in merged}

    # 2. Staging models fed by the seeded bronze (union across fixtures).
    staging, silver = dbt_runner.derive_selectors(touched)
    if staging:
        dbt_runner.build(" ".join(f"+{m}" for m in staging), worker_ctx=worker_ctx)

    # 3. Connector enrich steps (once, over every seeded source_id).
    touched_schemas = {schema for schema, _ in touched}
    ran_enrich_steps = []
    for step in enrich_runner.steps_for(touched_schemas):
        source_ids = enrich_runner.discover_source_ids(step, touched)
        if not source_ids:
            continue
        for schema, table in dbt_runner.enrich_output_tables(step.name):
            seeder.truncate_table(schema, table)
        enrich_runner.run(step, source_ids)
        ran_enrich_steps.append(step)

    # 4. Silver class models: those fed by seeded bronze + ephemeral enrich-fed targets.
    silver_set = set(silver)
    for step in ran_enrich_steps:
        silver_set.update(dbt_runner.ephemeral_silver_targets(step.name))
    if silver_set:
        dbt_runner.build(" ".join(sorted(silver_set)), worker_ctx=worker_ctx)

    # 5. Realign gold views to the now-real, populated silver — ONCE. (the
    #    e2e-migrate step created them against the placeholders dbt just dropped+rebuilt.)
    apply_gold_migrations(seeder.cfg)

    # 6. Refresh refreshable MVs ONCE.
    refresh_intermediates(seeder.cfg)
    LOG.info("seed-once world built: %d staging, %d silver, %d enrich steps",
             len(staging), len(silver_set), len(ran_enrich_steps))
