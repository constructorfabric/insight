"""Pruning the history snapshots built over bronze the generators rewrite.

A snapshot refuses a dbt full refresh: in production it is the only record of
earlier versions. A stand is not an archive, so once a generator has rewritten a
snapshot's bronze, the seed drops the versions of entities that bronze no longer
holds. Entities it still holds keep their versions, so a re-seed does not
re-observe them in the identity chain.
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING, NamedTuple

if TYPE_CHECKING:
    import clickhouse_connect.driver.client

LOG = logging.getLogger("seed.generators")

Relation = tuple[str, str]


class KeySource(NamedTuple):
    """A bronze relation and the expression that yields a snapshot key from its rows."""

    bronze: Relation
    key: str = "unique_key"


_JIRA_ISSUE_KEY = (
    "concat(COALESCE(tenant_id, ''), '-', COALESCE(source_id, ''), '-', COALESCE(jira_id, ''))"
)

#: Every snapshot over bronze the generators rewrite, with the bronze its keys come from.
SEEDED_SNAPSHOTS: dict[Relation, tuple[KeySource, ...]] = {
    ("staging", "bamboohr__employees_snapshot"): (KeySource(("bronze_bamboohr", "employees")),),
    ("staging", "chatgpt_team__seats_snapshot"): (
        KeySource(("bronze_chatgpt_team", "chatgpt_team_seats")),
    ),
    ("staging", "jira__comment_snapshot"): (KeySource(("bronze_jira", "jira_comments")),),
    ("staging", "jira__issue_availability_snapshot"): (
        KeySource(("bronze_jira", "jira_issue_census")),
        KeySource(("bronze_jira", "jira_issue"), _JIRA_ISSUE_KEY),
        KeySource(("bronze_jira", "jira_issue_keys"), _JIRA_ISSUE_KEY),
    ),
    ("staging", "jira__project_visibility_snapshot"): (
        KeySource(("bronze_jira", "jira_project_visibility")),
    ),
    ("staging", "jira__users_snapshot"): (KeySource(("bronze_jira", "jira_user")),),
    ("staging", "jira__worklog_snapshot"): (KeySource(("bronze_jira", "jira_worklogs")),),
    ("staging", "outline__users_snapshot"): (KeySource(("bronze_outline", "wiki_users")),),
    ("staging", "slack__users_snapshot"): (
        KeySource(
            ("bronze_slack", "users_details"), "concat(tenant_id, '-', source_id, '-', user_id)"
        ),
    ),
}


def prune_statement(snapshot: Relation, sources: tuple[KeySource, ...]) -> str:
    """Delete one tenant's versions whose key none of `sources` holds."""
    schema, table = snapshot
    held = " UNION ALL ".join(
        f"SELECT {source.key} FROM `{source.bronze[0]}`.`{source.bronze[1]}` "
        "WHERE tenant_id = {tenant:String}"
        for source in sources
    )
    return (
        f"ALTER TABLE `{schema}`.`{table}` DELETE "
        f"WHERE tenant_id = {{tenant:String}} AND unique_key NOT IN ({held})"
    )


def _exists(client: clickhouse_connect.driver.client.Client, relation: Relation) -> bool:
    schema, table = relation
    return bool(int(str(client.command(f"EXISTS TABLE `{schema}`.`{table}`"))))


def prune_to_bronze(
    client: clickhouse_connect.driver.client.Client,
    snapshot: Relation,
    tenant_uuid: str,
) -> None:
    """Keep only the versions of entities the snapshot's bronze holds for this tenant."""
    if not _exists(client, snapshot):
        return

    sources = tuple(
        source for source in SEEDED_SNAPSHOTS[snapshot] if _exists(client, source.bronze)
    )
    if not sources:
        return

    client.command(
        prune_statement(snapshot, sources),
        parameters={"tenant": tenant_uuid},
        settings={"mutations_sync": 2},
    )
    LOG.info(
        "%s.%s: pruned to the entities %s hold",
        *snapshot,
        ", ".join(".".join(source.bronze) for source in sources),
    )
