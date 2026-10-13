"""Move each table by copy, check, swap — and undo it.

INVARIANT: nothing writes to these tables while this runs (connectors and dbt
stopped). Each step re-reads the original and stops if it changed under it.
"""

from __future__ import annotations

import json
from collections.abc import Callable
from dataclasses import asdict, dataclass
from pathlib import Path

from tenant_migration import keys
from tenant_migration.clickhouse import ClickHouse
from tenant_migration.inventory import Table
from tenant_migration.plan import Plan, TablePlan

COPY, BACKUP, REJECTED, RESTORE = "__tenant_copy", "__tenant_backup", "__tenant_rejected", "__tenant_restore"
MANIFEST = "manifest.json"


class MigrationError(RuntimeError):
    pass


@dataclass(frozen=True)
class Measure:
    """What a copy must preserve: rows, or keys where merges may collapse rows."""

    rows: int
    keys: int


def measure(ch: ClickHouse, table: Table, suffix: str = "", key: str | None = None) -> Measure:
    key = key or table.columns.key
    keys_sql = f"uniqExact({key})" if key else "0"
    ((rows, unique),) = ch.rows(f"SELECT count(), {keys_sql} FROM {table.sql(suffix)}")
    return Measure(rows=int(rows), keys=int(unique))


def _same(table: Table, expected: Measure, actual: Measure) -> bool:
    """Keys on a replacing table, whose merges may collapse rows at any time; rows elsewhere."""
    if table.replacing and table.columns.key:
        return actual.keys == expected.keys
    return actual.rows == expected.rows


def _replacements(table: Table, tenants: keys.Tenants) -> str:
    columns = table.columns
    replaced = [f"{keys.new_tenant(columns.tenant, tenants)} AS {columns.tenant}"]
    if columns.key:
        replaced.append(f"{keys.rewritten_key(columns, tenants)} AS {columns.key}")
    return ", ".join(replaced)


def dump(ch: ClickHouse, plan: Plan, directory: Path) -> None:
    """DDL and rows of every table the plan changes, before anything changes."""
    directory.mkdir(parents=True, exist_ok=False)
    entries = []
    for t in plan.changing:
        table = t.table
        (directory / f"{table.relation}.sql").write_text(
            ch.execute(f"SHOW CREATE TABLE {table.sql()} FORMAT TabSeparatedRaw")
        )
        rows = measure(ch, table).rows
        ch.dump(f"SELECT * FROM {table.sql()}", directory / f"{table.relation}.native.gz")
        entries.append({"relation": table.relation, "rows": rows})
    manifest = {"tenant": plan.tenants.new, "old_tenant": plan.tenants.old, "tables": entries}
    (directory / MANIFEST).write_text(json.dumps(manifest, indent=2) + "\n")


def migrate_table(ch: ClickHouse, t: TablePlan, tenants: keys.Tenants) -> Measure:
    table = t.table
    new_key = keys.rewritten_key(table.columns, tenants) if table.columns.key else None
    before = measure(ch, table)
    expected = Measure(rows=before.rows, keys=measure(ch, table, key=new_key).keys if new_key else 0)

    ch.execute(f"DROP TABLE IF EXISTS {table.sql(COPY)}")
    ch.execute(f"CREATE TABLE {table.sql(COPY)} AS {table.sql()}")
    ch.execute(f"INSERT INTO {table.sql(COPY)} SELECT * REPLACE ({_replacements(table, tenants)}) FROM {table.sql()}")

    copied = measure(ch, table, COPY)
    if not _same(table, before, measure(ch, table)) or not _same(table, expected, copied):
        ch.execute(f"DROP TABLE IF EXISTS {table.sql(COPY)}")
        raise MigrationError(f"{table.relation}: copy holds {copied}, expected {expected} — the table is unchanged")

    ch.execute(f"EXCHANGE TABLES {table.sql()} AND {table.sql(COPY)}")
    live = measure(ch, table)
    if not _same(table, copied, live):
        ch.execute(f"EXCHANGE TABLES {table.sql()} AND {table.sql(COPY)}")
        raise MigrationError(f"{table.relation}: swapped-in table reads {live}, expected {copied} — swapped back")
    ch.execute(f"RENAME TABLE {table.sql(COPY)} TO {table.sql(BACKUP)}")
    return live


def apply(
    ch: ClickHouse, plan: Plan, dump_dir: Path, *, before_rewrite: list[Callable[[], None]]
) -> list[tuple[str, Measure]]:
    if plan.refusals:
        raise MigrationError("refusing: " + "; ".join(plan.refusals))
    dump(ch, plan, dump_dir)
    for step in before_rewrite:
        step()
    return [(t.table.relation, migrate_table(ch, t, plan.tenants)) for t in plan.changing]


def _work_tables(ch: ClickHouse, suffix: str) -> list[tuple[str, str]]:
    rows = ch.rows(f"SELECT database, name FROM system.tables WHERE endsWith(name, '{suffix}') ORDER BY database, name")
    return [(database, name[: -len(suffix)]) for database, name in rows]


def _sql(database: str, name: str) -> str:
    return f"`{database}`.`{name}`"


def rollback(ch: ClickHouse) -> list[str]:
    """Put every backed-up original back; the migrated table is kept as rejected."""
    restored = []
    for database, name in _work_tables(ch, BACKUP):
        live, backup, rejected = _sql(database, name), _sql(database, name + BACKUP), _sql(database, name + REJECTED)
        if ch.scalar(f"SELECT count() FROM system.tables WHERE database = '{database}' AND name = '{name}{REJECTED}'"):
            raise MigrationError(f"{database}.{name}{REJECTED} exists — finalize before rolling back again")
        ch.execute(f"EXCHANGE TABLES {live} AND {backup}")
        ch.execute(f"RENAME TABLE {backup} TO {rejected}")
        restored.append(f"{database}.{name}")
    return restored


def finalize(ch: ClickHouse) -> list[str]:
    """Drop the originals a migration kept, and the tables a rollback or restore set aside."""
    dropped = []
    for suffix in (BACKUP, REJECTED):
        for database, name in _work_tables(ch, suffix):
            ch.execute(f"DROP TABLE {_sql(database, name + suffix)}")
            dropped.append(f"{database}.{name}{suffix}")
    return dropped


def restore(ch: ClickHouse, dump_dir: Path) -> list[str]:
    """Load each dumped table into a fresh copy, check it, and swap it in."""
    manifest = json.loads((dump_dir / MANIFEST).read_text())
    restored = []
    for entry in manifest["tables"]:
        database, name = entry["relation"].split(".", 1)
        live, copy, rejected = _sql(database, name), _sql(database, name + RESTORE), _sql(database, name + REJECTED)
        if ch.scalar(f"SELECT count() FROM system.tables WHERE database = '{database}' AND name = '{name}{REJECTED}'"):
            raise MigrationError(f"{database}.{name}{REJECTED} exists — finalize before restoring")
        ch.execute(f"DROP TABLE IF EXISTS {copy}")
        ch.execute(f"CREATE TABLE {copy} AS {live}")
        ch.load(copy, dump_dir / f"{entry['relation']}.native.gz")
        loaded = ch.scalar(f"SELECT count() FROM {copy}")
        if loaded != entry["rows"]:
            ch.execute(f"DROP TABLE IF EXISTS {copy}")
            raise MigrationError(f"{entry['relation']}: dump loads {loaded} row(s), the manifest says {entry['rows']}")
        ch.execute(f"EXCHANGE TABLES {live} AND {copy}")
        ch.execute(f"RENAME TABLE {copy} TO {rejected}")
        restored.append(entry["relation"])
    return restored


def describe(measures: list[tuple[str, Measure]]) -> str:
    return "\n".join(f"{relation}: {json.dumps(asdict(m))}" for relation, m in measures)
