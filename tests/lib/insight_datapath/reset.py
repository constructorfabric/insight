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

Identity is never truncated whatever the instance: persons-sync swaps
`identity_persons` wholesale, and `identity_inputs` is rebuilt from scratch by each
spec's own full refresh. Truncating either would unresolve the persona the suite
authenticates as before the pipeline could put it back.
"""

from __future__ import annotations

import logging
from collections.abc import Iterable

from insight_datapath import clickhouse as ch
from insight_datapath.instance import InstanceConfig

LOG = logging.getLogger("datapath.reset")

Relation = tuple[str, str]

#: Mirrors `insight_seed.generators.insert.RESET_TARGETS`; a divergence fails
#: `tests/datapath/meta/test_reset.py`.
SEED_OWNED: frozenset[Relation] = frozenset(
    {
        ("bronze_bamboohr", "employees"),
        ("bronze_bitbucket_cloud", "repositories"),
        ("bronze_chatgpt_team", "chatgpt_team_chat_activity"),
        ("bronze_chatgpt_team", "chatgpt_team_seats"),
        ("bronze_claude_enterprise", "claude_enterprise_users"),
        ("bronze_claude_team", "claude_team_code_metrics"),
        ("bronze_claude_team", "claude_team_code_metrics_org"),
        ("bronze_claude_team", "claude_team_overage_spend"),
        ("bronze_claude_team_invoices", "claude_team_invoice_lines"),
        ("bronze_cursor", "cursor_daily_usage"),
        ("bronze_github", "commit_authors"),
        ("bronze_github", "commits"),
        ("bronze_github", "deployment_statuses"),
        ("bronze_github", "deployments"),
        ("bronze_github", "file_changes"),
        ("bronze_github", "pull_request_commits"),
        ("bronze_github", "pull_request_diff_stats"),
        ("bronze_github", "pull_requests"),
        ("bronze_github", "repositories"),
        ("bronze_github", "workflow_runs"),
        ("bronze_gitlab", "repositories"),
        ("bronze_hubspot", "deals"),
        ("bronze_hubspot", "engagements_calls"),
        ("bronze_hubspot", "engagements_emails"),
        ("bronze_hubspot", "engagements_meetings"),
        ("bronze_hubspot", "owners"),
        ("bronze_jira", "jira_fields"),
        ("bronze_jira", "jira_issue"),
        ("bronze_jira", "jira_issue_census"),
        ("bronze_jira", "jira_issue_history"),
        ("bronze_jira", "jira_issue_keys"),
        ("bronze_jira", "jira_issuetypes"),
        ("bronze_jira", "jira_project_visibility"),
        ("bronze_jira", "jira_projects"),
        ("bronze_jira", "jira_statuses"),
        ("bronze_jira", "jira_user"),
        ("bronze_jira", "jira_worklogs"),
        ("bronze_m365", "email_activity"),
        ("bronze_m365", "teams_activity"),
        ("bronze_outline", "wiki_comments"),
        ("bronze_outline", "wiki_page_versions"),
        ("bronze_outline", "wiki_pages"),
        ("bronze_outline", "wiki_spaces"),
        ("bronze_outline", "wiki_users"),
        ("bronze_slack", "users_details"),
        ("bronze_zendesk", "support_agents"),
        ("bronze_zendesk", "support_ticket_events"),
        ("bronze_zendesk", "zendesk_satisfaction_ratings"),
        ("bronze_zoom", "meetings"),
        ("bronze_zoom", "participants"),
        ("config", "field_value_defaults"),
        ("config", "field_value_map"),
        ("silver", "class_ai_assistant_usage"),
        ("silver", "class_ai_dev_usage"),
        ("silver", "class_ai_invoice"),
        ("silver", "class_ai_overage"),
        ("silver", "class_ai_overage_daily"),
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
        ("silver", "class_hr_working_hours"),
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
        ("staging", "chatgpt_team__ai_assistant_usage"),
        ("staging", "claude_enterprise__ai_assistant_usage"),
        ("staging", "claude_team__ai_dev_usage"),
        ("staging", "claude_team__ai_invoice"),
        ("staging", "claude_team__ai_overage"),
        ("staging", "claude_team__ai_overage_daily"),
        ("staging", "cursor__ai_dev_usage"),
        ("staging", "github__ci_runs"),
        ("staging", "github__commits"),
        ("staging", "github__deployment_events"),
        ("staging", "github__deployments"),
        ("staging", "github__file_changes"),
        ("staging", "github__pull_requests"),
        ("staging", "github__pull_requests_commits"),
        ("staging", "github__repositories"),
        ("staging", "gitlab__repositories"),
        ("staging", "hubspot__crm_activities"),
        ("staging", "hubspot__crm_deals"),
        ("staging", "hubspot__crm_users"),
        ("staging", "jira__field_history_derived"),
        ("staging", "jira__task_worklogs"),
        ("staging", "m365__collab_chat_activity"),
        ("staging", "m365__collab_email_activity"),
        ("staging", "m365__collab_meeting_activity"),
        ("staging", "slack__collab_chat_activity"),
        ("staging", "zendesk__support_activity"),
        ("staging", "zendesk__support_agent"),
        ("staging", "zendesk__support_event"),
        ("staging", "zoom__collab_meeting_activity"),
        ("staging", "zoom__meeting_sessions"),
    }
)

SERVICE_OWNED: frozenset[Relation] = frozenset(
    {
        ("identity", "identity_persons"),
        ("identity", "identity_inputs"),
    }
)

#: The seed step that leaves the warehouse empty.
WAREHOUSE_IS_OURS: frozenset[str] = frozenset({"identity"})

#: `config` is fixture data too: specs seed task_field_roles / task_value_map there,
#: and rows outliving a session feed the next session's closure build (e.g.
#: github__task_statuses reads them), whose fresh `_version` then starves the next
#: spec's own rows out of the silver watermark.
FIXTURE_DATABASES = "^(bronze_.*|staging|silver|config)$"


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
