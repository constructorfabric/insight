"""Pruning the history snapshots built over bronze the generators rewrite.

A snapshot refuses a dbt full refresh: in production it is the only record of
earlier versions. A stand is not an archive, so once the generators have
rewritten bronze, the seed drops the versions of entities the snapshot's own
source no longer holds. Entities it still holds keep their versions, so a
re-seed does not re-observe them in the identity chain.

A snapshot's unique_key is whatever its source relation carries, and a source
built by a model may compute it from several bronze tables. The keys are
therefore read from that source relation, after it is rebuilt over the new
bronze and before the snapshot appends to it.
"""

from __future__ import annotations

import logging
from collections.abc import Iterable, Iterator, Sequence
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    import clickhouse_connect.driver.client

LOG = logging.getLogger("seed.generators")

Relation = tuple[str, str]

_DELETE_BATCH_KEYS = 1000

#: Every snapshot over bronze the generators rewrite, with the relation its
#: `source_ref` names.
SEEDED_SNAPSHOTS: dict[Relation, Relation] = {
    ("staging", "bamboohr__employees_snapshot"): ("bronze_bamboohr", "employees"),
    ("staging", "chatgpt_team__seats_snapshot"): ("staging", "chatgpt_team__seats_latest"),
    ("staging", "jira__comment_snapshot"): ("staging", "jira__comment_state"),
    ("staging", "jira__issue_availability_snapshot"): (
        "staging",
        "jira__issue_availability_state",
    ),
    ("staging", "jira__project_visibility_snapshot"): ("staging", "jira__project_visibility_state"),
    ("staging", "jira__users_snapshot"): ("bronze_jira", "jira_user"),
    ("staging", "jira__worklog_snapshot"): ("staging", "jira__worklog_state"),
    ("staging", "outline__users_snapshot"): ("bronze_outline", "wiki_users"),
    ("staging", "slack__users_snapshot"): ("staging", "slack__users_latest"),
}


def sources_to_rebuild(present: Iterable[Relation]) -> list[str]:
    """The dbt models the given snapshots read, which must be rebuilt before pruning."""
    sources = (SEEDED_SNAPSHOTS[snapshot] for snapshot in present)
    return sorted({table for schema, table in sources if not schema.startswith("bronze_")})


def source_models() -> list[str]:
    """Every dbt model a seeded snapshot reads."""
    return sources_to_rebuild(SEEDED_SNAPSHOTS)


def existing_snapshots(client: clickhouse_connect.driver.client.Client) -> list[Relation]:
    """The seeded snapshots a previous seed has already built."""
    return [
        snapshot
        for snapshot in sorted(SEEDED_SNAPSHOTS)
        if int(str(client.command(f"EXISTS TABLE `{snapshot[0]}`.`{snapshot[1]}`")))
    ]


def stale_keys_query(snapshot: Relation, source: Relation) -> str:
    """One tenant's snapshot keys that its source no longer holds."""
    schema, table = snapshot
    source_schema, source_table = source
    return (
        f"SELECT DISTINCT unique_key FROM `{schema}`.`{table}` "
        "WHERE tenant_id = {tenant:String} AND unique_key NOT IN ("
        f"SELECT unique_key FROM `{source_schema}`.`{source_table}` FINAL "
        "WHERE tenant_id = {tenant:String})"
    )


def delete_statement(snapshot: Relation) -> str:
    """Delete one tenant's versions whose key is among `keys`."""
    schema, table = snapshot
    return (
        f"ALTER TABLE `{schema}`.`{table}` DELETE "
        "WHERE tenant_id = {tenant:String} AND has({keys:Array(String)}, unique_key)"
    )


def batches(keys: Sequence[str], size: int = _DELETE_BATCH_KEYS) -> Iterator[list[str]]:
    for start in range(0, len(keys), size):
        yield list(keys[start : start + size])


def prune_to_source(
    client: clickhouse_connect.driver.client.Client,
    snapshot: Relation,
    tenant_uuid: str,
) -> int:
    """Drop the versions of entities the snapshot's source no longer holds for this tenant."""
    schema, table = snapshot
    source = SEEDED_SNAPSHOTS[snapshot]

    if not int(str(client.command(f"EXISTS TABLE `{schema}`.`{table}`"))):
        return 0

    stale = [
        str(row[0])
        for row in client.query(
            stale_keys_query(snapshot, source), parameters={"tenant": tenant_uuid}
        ).result_rows
    ]
    for keys in batches(stale):
        client.command(
            delete_statement(snapshot),
            parameters={"tenant": tenant_uuid, "keys": keys},
            settings={"mutations_sync": 2},
        )

    LOG.info("%s.%s: pruned %d entities %s.%s no longer holds", schema, table, len(stale), *source)
    return len(stale)


def prune_seeded_snapshots(
    client: clickhouse_connect.driver.client.Client,
    tenant_uuid: str,
    present: Iterable[Relation],
) -> None:
    for snapshot in present:
        prune_to_source(client, snapshot, tenant_uuid)
