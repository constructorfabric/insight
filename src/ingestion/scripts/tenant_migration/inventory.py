"""Which tables hold a tenant the migration moves, and in which columns.

Bronze, operator config and the sync ledger are found by database; the dbt
relations a rebuild cannot reproduce come from `history-tables.txt`, packaged
beside this module.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from importlib import (
    resources,
)  # nosemgrep: python.lang.compatibility.python37.python37-compatibility-importlib2 — requires-python >= 3.12

from tenant_migration.clickhouse import ClickHouse
from tenant_migration.keys import Columns

HISTORY_TABLES = "history-tables.txt"
IDENTIFIER = re.compile(r"^[A-Za-z0-9_]+$")
COPY, BACKUP, REJECTED, RESTORE = "__tenant_copy", "__tenant_backup", "__tenant_rejected", "__tenant_restore"
WORK_SUFFIXES = (COPY, BACKUP, REJECTED, RESTORE)

TENANT_COLUMNS = ("tenant_id", "insight_tenant_id")
SOURCE_COLUMNS = ("source_id", "insight_source_id")
KEY_COLUMN = "unique_key"
STRING_TYPES = ("String", "Nullable(String)", "LowCardinality(String)", "LowCardinality(Nullable(String))")


@dataclass(frozen=True)
class Relation:
    """A table by database and name, both plain identifiers."""

    database: str
    name: str

    def __post_init__(self) -> None:
        if not (IDENTIFIER.match(self.database) and IDENTIFIER.match(self.name)):
            raise ValueError(f"not a plain identifier: {self.database}.{self.name}")

    @classmethod
    def parse(cls, relation: str) -> Relation:
        database, name = relation.split(".", 1)
        return cls(database, name)

    def sql(self, suffix: str = "") -> str:
        return f"`{self.database}`.`{self.name}{suffix}`"

    def __str__(self) -> str:
        return f"{self.database}.{self.name}"


@dataclass(frozen=True)
class Table:
    database: str
    name: str
    engine: str
    columns: Columns

    @property
    def rel(self) -> Relation:
        return Relation(self.database, self.name)

    @property
    def relation(self) -> str:
        return str(self.rel)

    def sql(self, suffix: str = "") -> str:
        return self.rel.sql(suffix)

    @property
    def replacing(self) -> bool:
        return "Replacing" in self.engine


@dataclass(frozen=True)
class Unfit:
    """A table the migration cannot read safely; refused when it holds rows."""

    relation: str
    reason: str


def history_relations() -> list[tuple[str, str]]:
    text = resources.files("tenant_migration").joinpath(HISTORY_TABLES).read_text(encoding="utf-8")
    lines = (line.strip() for line in text.splitlines())
    relations = [tuple(line.split(".", 1)) for line in lines if line and not line.startswith("#")]
    if not relations:
        raise SystemExit(f"no relations listed in {HISTORY_TABLES}")
    return [(database, name) for database, name in relations]


def _scope_sql(history: list[tuple[str, str]]) -> str:
    pairs = ", ".join(f"('{database}', '{name}')" for database, name in history)
    work = " AND ".join(f"NOT endsWith(t.name, '{suffix}')" for suffix in WORK_SUFFIXES)
    return (
        f"(t.database LIKE 'bronze\\\\_%' OR t.database IN ('config', 'ingestion_history') "
        f"OR (t.database, t.name) IN ({pairs})) AND t.engine LIKE '%MergeTree' AND {work}"
    )


def classify(database: str, name: str, engine: str, types: dict[str, str]) -> Table | Unfit:
    relation = f"{database}.{name}"
    if not (IDENTIFIER.match(database) and IDENTIFIER.match(name)):
        return Unfit(relation, "a name that is not a plain identifier")
    tenant = next((c for c in TENANT_COLUMNS if c in types), None)
    source = next((c for c in SOURCE_COLUMNS if c in types), None)
    key = KEY_COLUMN if KEY_COLUMN in types else None

    if tenant is None:
        return Unfit(relation, "no tenant column")
    if types[tenant] not in STRING_TYPES:
        return Unfit(relation, f"{tenant} is {types[tenant]}, not a string")
    if key is not None and source is None:
        return Unfit(relation, "a key but no source column to tell where its tenant ends")
    return Table(database, name, engine, Columns(tenant=tenant, source=source, key=key))


def discover(ch: ClickHouse, history: list[tuple[str, str]]) -> list[Table | Unfit]:
    rows = ch.rows(
        f"""
        SELECT t.database, t.name, t.engine, c.name, c.type
        FROM system.tables AS t
        INNER JOIN system.columns AS c ON c.database = t.database AND c.table = t.name
        WHERE {_scope_sql(history)}
        ORDER BY t.database, t.name, c.position
        """
    )
    tables: dict[tuple[str, str, str], dict[str, str]] = {}
    for database, name, engine, column, column_type in rows:
        tables.setdefault((database, name, engine), {})[column] = column_type
    return [classify(database, name, engine, types) for (database, name, engine), types in tables.items()]
