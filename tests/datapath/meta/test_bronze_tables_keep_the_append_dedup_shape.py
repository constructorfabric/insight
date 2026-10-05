"""The shape every `bronze_*` relation the connectors created must hold.

Bronze relations are owned by the Airbyte destination running in `append_dedup` mode:
a `ReplacingMergeTree` versioned by the extraction stamp and sorted by a non-nullable
`unique_key`. A table carrying the older `MergeTree ORDER BY _airbyte_raw_id` shape
keeps every re-read of a source row as its own row instead of collapsing it, so a
reader that does not dedup counts one record once per sync that saw it.

Nothing else in this suite sees that: the sibling test compares column sets, and a
legacy-shaped table has exactly the same columns as a correct one. Only a full
rebuild would notice, and only by producing wrong numbers.

Read off the warehouse the session fixture built, which is the destination's own
output — there is no committed bronze DDL to read instead, and asserting against one
would only restate what some earlier dump happened to hold.

Scope is the `bronze_*` databases alone. `silver`, `staging`, `identity` and `insight`
relations have other owners and other legitimate shapes -- `allow_nullable_key` among
them -- and `insight.bronze_insert_events` is a view whose NAME carries the prefix
while its database does not, so the scope is read off the database, never the name.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

import pytest
from insight_datapath import clickhouse as ch
from insight_datapath.instance import InstanceConfig

BRONZE_DATABASE_PATTERN = "bronze\\_%"
DEDUP_ENGINE = "ReplacingMergeTree(_airbyte_extracted_at)"
DEDUP_SORT_KEY = "unique_key"
DEDUP_KEY_TYPE = "String"
NULLABLE_KEY_SETTING = "allow_nullable_key"


@dataclass(frozen=True)
class Table:
    """One bronze table as ClickHouse itself describes it."""

    name: str
    engine: str
    order_by: str
    unique_key_type: str | None
    engine_full: str


@pytest.fixture(scope="session")
def bronze_tables(instance_cfg: InstanceConfig, warehouse_schema: int) -> list[Table]:
    """Every table the connectors created, keyed by what the engine reports.

    `engine_full` carries the engine with its version argument, the sorting key and
    the table settings, so one row answers every rule below bar the key's type.
    """
    key_types = {
        f"{database}.{table}": column_type
        for database, table, column_type in ch.query(
            instance_cfg,
            "SELECT database, table, type FROM system.columns "
            f"WHERE database LIKE '{BRONZE_DATABASE_PATTERN}' AND name = '{DEDUP_SORT_KEY}'",
        )
    }
    rows = ch.query(
        instance_cfg,
        "SELECT database, name, engine_full, sorting_key FROM system.tables "
        f"WHERE database LIKE '{BRONZE_DATABASE_PATTERN}' "
        "AND engine NOT IN ('View', 'MaterializedView') ORDER BY database, name",
    )
    return [
        Table(
            name=f"{database}.{name}",
            engine=str(engine_full).split(" ORDER BY")[0].strip(),
            order_by=str(sorting_key).strip(),
            unique_key_type=key_types.get(f"{database}.{name}"),
            engine_full=str(engine_full),
        )
        for database, name, engine_full, sorting_key in rows
    ]


def _offenders(tables: list[Table], deviation: Callable[[Table], str | None]) -> list[str]:
    """Every bronze table the rule rejects, each carrying what it actually declares."""
    reported = ((table, deviation(table)) for table in tables)
    return sorted(f"{table.name} {report}" for table, report in reported if report)


def test_the_connectors_created_bronze_tables(bronze_tables: list[Table]) -> None:
    """A warehouse this file cannot read would make every rule below vacuous."""
    assert len(bronze_tables) > 100, f"only {len(bronze_tables)} bronze tables were created"
    databases = {table.name.split(".", 1)[0] for table in bronze_tables}
    assert {"bronze_github", "bronze_jira"} < databases


def test_a_bronze_table_is_a_replacing_merge_tree_versioned_by_the_extraction_stamp(
    bronze_tables: list[Table],
) -> None:
    """A plain `MergeTree` never collapses anything, so every re-read of a source row
    survives as a row of its own."""
    wrong = _offenders(
        bronze_tables,
        lambda table: f"has ENGINE = {table.engine!r}" if table.engine != DEDUP_ENGINE else None,
    )
    assert wrong == [], f"should declare ENGINE = {DEDUP_ENGINE}: {wrong}"


def test_a_bronze_table_is_sorted_by_its_unique_key(bronze_tables: list[Table]) -> None:
    """Replacement collapses rows sharing the sort key. Sorting by `_airbyte_raw_id` --
    minted fresh on every read -- means no two rows ever share one."""
    wrong = _offenders(
        bronze_tables,
        lambda table: (
            f"has ORDER BY {table.order_by!r}" if table.order_by != DEDUP_SORT_KEY else None
        ),
    )
    assert wrong == [], f"should declare ORDER BY {DEDUP_SORT_KEY}: {wrong}"


def test_a_bronze_table_declares_a_non_nullable_unique_key(bronze_tables: list[Table]) -> None:
    """A nullable sort key lets every row whose key is null collapse into one."""
    wrong = _offenders(
        bronze_tables,
        lambda table: (
            f"declares unique_key {table.unique_key_type!r}"
            if table.unique_key_type != DEDUP_KEY_TYPE
            else None
        ),
    )
    assert wrong == [], f"should declare `unique_key` {DEDUP_KEY_TYPE}: {wrong}"


def test_a_bronze_table_never_permits_a_nullable_sort_key(bronze_tables: list[Table]) -> None:
    """The setting is what lets a legacy-shaped table exist at all; while it is set,
    the non-nullable key above can be reverted without the engine complaining."""
    wrong = _offenders(
        bronze_tables,
        lambda table: (
            f"sets {NULLABLE_KEY_SETTING}" if NULLABLE_KEY_SETTING in table.engine_full else None
        ),
    )
    assert wrong == [], f"should not set {NULLABLE_KEY_SETTING}: {wrong}"
