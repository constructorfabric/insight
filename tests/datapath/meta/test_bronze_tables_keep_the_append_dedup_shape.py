"""The shape every `bronze_*` relation in the committed DDL snapshot must hold.

Bronze relations are owned by the Airbyte destination running in `append_dedup` mode:
a `ReplacingMergeTree` versioned by the extraction stamp and sorted by a non-nullable
`unique_key`. A table carrying the older `MergeTree ORDER BY _airbyte_raw_id` shape
keeps every re-read of a source row as its own row instead of collapsing it, so a
reader that does not dedup counts one record once per sync that saw it.

Nothing else in this suite sees that: the sibling snapshot test compares column sets,
and a legacy-shaped table has exactly the same columns as a correct one. Only a full
rebuild would notice, and only by producing wrong numbers.

Scope is the `bronze_*` databases alone. `silver`, `staging`, `identity` and `insight`
relations have other owners and other legitimate shapes -- `allow_nullable_key` among
them -- and `insight.bronze_insert_events` is a view whose NAME carries the prefix
while its database does not, so the scope is read off the database, never the name.
"""

from __future__ import annotations

import re
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
DDL_DIR = REPO_ROOT / "src/ingestion/scripts/connectors-ddl"

BRONZE_DATABASE_PREFIX = "bronze_"
DEDUP_ENGINE = "ReplacingMergeTree(_airbyte_extracted_at)"
DEDUP_SORT_KEY = "unique_key"
DEDUP_KEY_TYPE = "String"
NULLABLE_KEY_SETTING = "allow_nullable_key"

_CREATE = re.compile(
    r"CREATE TABLE IF NOT EXISTS\s+(?P<name>[\w.]+)\s*"
    r"\((?P<body>.*?)\)\s*(?P<tail>ENGINE\s*=.*?)\n;",
    re.DOTALL,
)
_COLUMN = re.compile(r"^\s*`(?P<column>[^`]+)`\s+(?P<type>.+?),?\s*$")
_ENGINE = re.compile(r"ENGINE\s*=\s*(?P<engine>.+)")
_ORDER_BY = re.compile(r"^ORDER BY\s+(?P<keys>.+)$", re.MULTILINE)


@dataclass(frozen=True)
class Table:
    """One `CREATE TABLE` of the generated snapshot, as the snapshot spells it."""

    name: str
    source: str
    engine: str
    order_by: str | None
    unique_key_type: str | None
    clause: str

    @property
    def database(self) -> str:
        return self.name.split(".", 1)[0]


def _table(name: str, body: str, clause: str, source: str) -> Table:
    engine = _ENGINE.search(clause)
    order_by = _ORDER_BY.search(clause)
    columns = {
        match.group("column"): match.group("type").strip()
        for match in (_COLUMN.match(line) for line in body.splitlines())
        if match
    }

    return Table(
        name=name,
        source=source,
        engine=engine.group("engine").strip() if engine else "",
        order_by=order_by.group("keys").strip() if order_by else None,
        unique_key_type=columns.get(DEDUP_SORT_KEY),
        clause=clause,
    )


def _bronze_tables() -> list[Table]:
    """Every table the generated snapshot declares in a `bronze_*` database."""
    tables = [
        _table(create.group("name"), create.group("body"), create.group("tail"), sql.name)
        for sql in sorted(DDL_DIR.glob("*.sql"))
        for create in _CREATE.finditer(sql.read_text(encoding="utf-8"))
    ]
    return [table for table in tables if table.database.startswith(BRONZE_DATABASE_PREFIX)]


BRONZE = _bronze_tables()


def _offenders(deviation: Callable[[Table], str | None]) -> list[str]:
    """Every bronze table the rule rejects, each carrying what it actually declares."""
    reported = ((table, deviation(table)) for table in BRONZE)
    return sorted(f"{table.source}: {table.name} {report}" for table, report in reported if report)


def test_the_snapshot_yields_bronze_tables() -> None:
    """A snapshot this file cannot read would make every rule below vacuous."""
    assert len(BRONZE) > 100, f"only {len(BRONZE)} bronze tables parsed out of {DDL_DIR}"
    assert {"bronze_github", "bronze_jira"} < {table.database for table in BRONZE}


def test_a_bronze_table_is_a_replacing_merge_tree_versioned_by_the_extraction_stamp() -> None:
    """A plain `MergeTree` never collapses anything, so every re-read of a source row
    survives as a row of its own."""
    wrong = _offenders(
        lambda table: f"has ENGINE = {table.engine!r}" if table.engine != DEDUP_ENGINE else None
    )
    assert wrong == [], f"should declare ENGINE = {DEDUP_ENGINE}: {wrong}"


def test_a_bronze_table_is_sorted_by_its_unique_key() -> None:
    """Replacement collapses rows sharing the sort key. Sorting by `_airbyte_raw_id` --
    minted fresh on every read -- means no two rows ever share one."""
    wrong = _offenders(
        lambda table: (
            f"has ORDER BY {table.order_by!r}" if table.order_by != DEDUP_SORT_KEY else None
        )
    )
    assert wrong == [], f"should declare ORDER BY {DEDUP_SORT_KEY}: {wrong}"


def test_a_bronze_table_declares_a_non_nullable_unique_key() -> None:
    """A nullable sort key lets every row whose key is null collapse into one."""
    wrong = _offenders(
        lambda table: (
            f"declares unique_key {table.unique_key_type!r}"
            if table.unique_key_type != DEDUP_KEY_TYPE
            else None
        )
    )
    assert wrong == [], f"should declare `unique_key` {DEDUP_KEY_TYPE}: {wrong}"


def test_a_bronze_table_never_permits_a_nullable_sort_key() -> None:
    """The setting is what lets a legacy-shaped table exist at all; while it is set,
    the non-nullable key above can be reverted without the engine complaining."""
    wrong = _offenders(
        lambda table: (
            f"sets {NULLABLE_KEY_SETTING}" if NULLABLE_KEY_SETTING in table.clause else None
        )
    )
    assert wrong == [], f"should not set {NULLABLE_KEY_SETTING}: {wrong}"
