"""Pruning the history snapshots built over bronze the generators rewrite.

A snapshot refuses a dbt full refresh: in production it is the only record of
earlier versions. A stand is not an archive, so once a generator has rewritten a
snapshot's bronze, the seed drops the versions of entities that bronze no longer
holds. Entities it still holds keep their versions, so a re-seed does not
re-observe them in the identity chain.
"""

from __future__ import annotations

import logging
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    import clickhouse_connect.driver.client

LOG = logging.getLogger("seed.generators")

Relation = tuple[str, str]

#: Every snapshot over bronze the generators rewrite, with the bronze it reads.
SEEDED_SNAPSHOTS: dict[Relation, Relation] = {
    ("staging", "bamboohr__employees_snapshot"): ("bronze_bamboohr", "employees"),
}


def prune_statement(snapshot: Relation) -> str:
    """Delete one tenant's versions whose key is not among `keys`."""
    schema, table = snapshot
    return (
        f"ALTER TABLE `{schema}`.`{table}` DELETE "
        "WHERE tenant_id = {tenant:String} AND NOT has({keys:Array(String)}, unique_key)"
    )


def prune_to_bronze(
    client: clickhouse_connect.driver.client.Client,
    snapshot: Relation,
    tenant_uuid: str,
) -> None:
    """Keep only the versions of entities the snapshot's bronze holds for this tenant."""
    schema, table = snapshot
    bronze_schema, bronze_table = SEEDED_SNAPSHOTS[snapshot]

    if not int(str(client.command(f"EXISTS TABLE `{schema}`.`{table}`"))):
        return

    keys = [
        str(row[0])
        for row in client.query(
            f"SELECT unique_key FROM `{bronze_schema}`.`{bronze_table}` "
            "WHERE tenant_id = {tenant:String}",
            parameters={"tenant": tenant_uuid},
        ).result_rows
    ]
    client.command(
        prune_statement(snapshot),
        parameters={"tenant": tenant_uuid, "keys": keys},
        settings={"mutations_sync": 2},
    )
    LOG.info(
        "%s.%s: pruned to the %d entities %s.%s holds",
        schema,
        table,
        len(keys),
        bronze_schema,
        bronze_table,
    )
