"""Clearing the warehouse between specs, and refusing to clear what is not ours.

A spec asserts against its own seed alone, so every relation the last spec wrote is
emptied before the next one seeds. Two rules shape which relations those are.

Only what a run actually wrote is cleared: the set comes from the ledger the seeder
and the model builds fill in, not from a list maintained by hand beside them.

And the warehouse has to be this suite's to clear. A stand seeded with a roster holds
its people in the same silver classes a spec builds, so clearing by layer would delete
the roster the caller authenticates as. Which relations those are is named below, and
whether any of them hold the stand's rows is a fact the instance reports: a run is
refused at the door unless the instance seeded identity alone.

Identity is never cleared whatever the instance. persons-sync swaps one of its
relations wholesale and the other carries the roster's own observations — emptying it
unresolves the persona the suite authenticates as.
"""

from __future__ import annotations

import logging
from collections.abc import Iterable

from insight_datapath import clickhouse as ch
from insight_datapath.instance import InstanceConfig

LOG = logging.getLogger("datapath.reset")

Relation = tuple[str, str]

#: Relations the stand's seeder writes and truncates
#: (`insight_seed.generators.insert.RESET_TARGETS`). Copied rather than imported: the
#: seeder is not a dependency of this project, and a divergence is caught by
#: `test_seed_owned_relations_match_the_seeder`.
SEED_OWNED: frozenset[Relation] = frozenset(
    {
        ("bronze_bamboohr", "employees"),
        ("bronze_bitbucket_cloud", "repositories"),
        ("bronze_claude_team_invoices", "claude_team_invoice_lines"),
        ("bronze_github", "deployment_statuses"),
        ("bronze_github", "deployments"),
        ("bronze_github", "repositories"),
        ("bronze_github", "workflow_runs"),
        ("bronze_gitlab", "projects"),
        ("silver", "class_ai_assistant_usage"),
        ("silver", "class_ai_dev_usage"),
        ("silver", "class_ai_invoice"),
        ("silver", "class_ai_overage"),
        ("silver", "class_collab_chat_activity"),
        ("silver", "class_collab_email_activity"),
        ("silver", "class_collab_meeting_activity"),
        ("silver", "class_crm_activities"),
        ("silver", "class_crm_deals"),
        ("silver", "class_crm_users"),
        ("silver", "class_focus_metrics"),
        ("silver", "class_git_ci_runs"),
        ("silver", "class_git_commits"),
        ("silver", "class_git_deployment_events"),
        ("silver", "class_git_deployments"),
        ("silver", "class_git_file_changes"),
        ("silver", "class_git_pull_requests"),
        ("silver", "class_git_pull_requests_commits"),
        ("silver", "class_git_repositories"),
        ("silver", "class_people"),
        ("silver", "class_support_activity"),
        ("silver", "class_task_field_history"),
        ("silver", "class_task_issuetypes"),
        ("silver", "class_task_statuses"),
        ("silver", "class_task_users"),
        ("silver", "class_task_worklogs"),
        ("silver", "class_wiki_activity"),
        ("silver", "class_wiki_engagement"),
        ("silver", "class_wiki_pages"),
        ("staging", "bitbucket_cloud__repositories"),
        ("staging", "claude_team__ai_invoice"),
        ("staging", "github__ci_runs"),
        ("staging", "github__deployment_events"),
        ("staging", "github__deployments"),
        ("staging", "github__repositories"),
        ("staging", "gitlab__repositories"),
    }
)

#: Written by the product, not by any suite.
SERVICE_OWNED: frozenset[Relation] = frozenset(
    {
        ("identity", "identity_persons"),
        ("identity", "identity_inputs"),
    }
)

#: The seed step that leaves the warehouse empty. Anything more and the stand's own
#: rows share the relations a spec builds.
WAREHOUSE_IS_OURS: frozenset[str] = frozenset({"identity"})

#: Databases whose every relation holds fixture-derived rows.
FIXTURE_DATABASES = "^(bronze_.*|staging|silver)$"


class SeededWarehouseError(RuntimeError):
    """The instance holds rows this suite would clear but does not own."""


def populated_relations(cfg: InstanceConfig) -> list[Relation]:
    """Every fixture-data relation that currently holds rows.

    Read from the warehouse rather than accumulated as tests run, so it covers what a
    failed build left behind as well as what a successful one wrote. `system.parts`
    lists the MergeTree family, which is the family that holds rows and the only one
    ClickHouse truncates.
    """
    rows = ch.query(
        cfg,
        f"""
        SELECT database, table
        FROM system.parts
        WHERE active
          AND match(database, '{FIXTURE_DATABASES}')
        GROUP BY database, table
        HAVING sum(rows) > 0
        ORDER BY database, table
        """,
    )
    return [(str(database), str(table)) for database, table in rows]


def refuse_a_seeded_warehouse(seeded: Iterable[str]) -> None:
    """Raise unless the instance's warehouse is this suite's to write.

    The stand's seeder fills the same silver classes a spec builds, so a suite that
    cleared them would delete the roster its own caller authenticates as.
    """
    extra = sorted(set(seeded) - WAREHOUSE_IS_OURS)
    if extra:
        raise SeededWarehouseError(
            f"this instance was seeded with {', '.join(extra)}, which fills "
            f"{len(SEED_OWNED)} of the relations a spec builds. Bring up an instance of "
            "its own with `test-stand minimal`, which seeds identity and leaves the "
            "warehouse empty."
        )


def clear(cfg: InstanceConfig, relations: Iterable[Relation]) -> int:
    """Empty each relation this suite owns, returning how many were cleared."""
    targets = sorted(set(relations) - SERVICE_OWNED)
    for database, table in targets:
        ch.execute(cfg, f"TRUNCATE TABLE IF EXISTS `{database}`.`{table}`")
    LOG.debug("reset: cleared %d relations", len(targets))
    return len(targets)


def session_floor(cfg: InstanceConfig) -> int:
    """Empty every fixture-data relation holding rows, once, before the first spec.

    The staging and silver models are incremental behind a watermark over their own
    target, and a fixture pins the value it seeds, so rows left by an earlier session
    would keep the fixture's own rows out and the assertions would read the older data.
    """
    populated = populated_relations(cfg)
    cleared = clear(cfg, populated)
    LOG.info("session floor: cleared %d populated relations", cleared)
    return cleared
