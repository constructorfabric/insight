"""Move each table by copy, check, swap — and recover from any step of it.

Which table holds the original is never inferred from a name or remembered in a
file: every rewritten copy carries the table comment MIGRATED, and every table
loaded from a dump carries RESTORED, from the moment it is created. A comment
travels with its table through EXCHANGE and RENAME, so rollback, finalize and
restore read the state of an interrupted run from the warehouse itself — even
when the reply to a swap was lost — and a second run of any of them finishes
what the first left.

INVARIANT: nothing writes to these tables while this runs (connectors and dbt
stopped). Each step re-reads the original and stops if it changed under it.
"""

from __future__ import annotations

import base64
import contextlib
import json
import re
from collections.abc import Callable
from dataclasses import asdict, dataclass
from pathlib import Path

from tenant_migration import ddl, keys
from tenant_migration.clickhouse import ClickHouse
from tenant_migration.inventory import BACKUP, COPY, REJECTED, RESTORE, Relation, Table
from tenant_migration.inventory import WORK_SUFFIXES as WORK
from tenant_migration.plan import Plan, TablePlan

MIGRATED = "tenant_migration: rewritten copy"
RESTORED = "tenant_migration: loaded from dump"
MANIFEST = "manifest.json"
CREATE_TABLE = re.compile(r"^CREATE TABLE \S+(?: UUID '[0-9A-Fa-f-]+')?")


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


def _same(by_keys: bool, expected: Measure, actual: Measure) -> bool:
    """Keys on a replacing table, whose merges may collapse rows at any time; rows elsewhere."""
    return actual.keys == expected.keys if by_keys else actual.rows == expected.rows


def _by_keys(table: Table) -> bool:
    return table.replacing and table.columns.key is not None


def _comments(ch: ClickHouse, rel: Relation) -> dict[str, str]:
    """suffix → table comment, for the relation and each of its work tables that exists."""
    names = ", ".join(f"'{rel.name}{suffix}'" for suffix in ("", *WORK))
    rows = ch.rows(
        f"SELECT name, base64Encode(comment) FROM system.tables WHERE database = '{rel.database}' AND name IN ({names})"
    )
    return {name[len(rel.name) :]: decode_comment(encoded) for name, encoded in rows}


def decode_comment(encoded: str) -> str:
    """A comment read through base64, so a tab or newline in it cannot split the row."""
    return base64.b64decode(encoded).decode("utf-8")


def _set_comment(ch: ClickHouse, rel: Relation, suffix: str, comment: str) -> None:
    ch.execute(ddl.set_comment(rel, suffix, comment))


def _replacements(table: Table, tenants: keys.Tenants) -> str:
    columns = table.columns
    replaced = [f"{keys.new_tenant(columns.tenant, tenants)} AS {columns.tenant}"]
    if columns.key:
        replaced.append(f"{keys.rewritten_key(columns, tenants)} AS {columns.key}")
    return ", ".join(replaced)


def dump(ch: ClickHouse, plan: Plan, directory: Path) -> None:
    """DDL, rows and comment of every table the plan changes, before anything changes."""
    directory.mkdir(parents=True, exist_ok=False)
    entries = []
    for t in plan.changing:
        table = t.table
        rel = Relation(table.database, table.name)
        (directory / f"{table.relation}.sql").write_text(ch.execute(ddl.show_create(table.rel), fmt="TabSeparatedRaw"))
        counted = measure(ch, table)
        ch.dump(f"SELECT * FROM {table.sql()}", directory / f"{table.relation}.native.gz")
        entries.append(
            {
                "relation": table.relation,
                "rows": counted.rows,
                "keys": counted.keys,
                "by_keys": _by_keys(table),
                "comment": _comments(ch, rel).get("", ""),
            }
        )
    manifest = {"tenant": plan.tenants.new, "old_tenant": plan.tenants.old, "tables": entries}
    (directory / MANIFEST).write_text(json.dumps(manifest, indent=2) + "\n")


def migrate_table(ch: ClickHouse, t: TablePlan, tenants: keys.Tenants) -> Measure:
    table = t.table
    rel = Relation(table.database, table.name)
    by_keys = _by_keys(table)
    new_key = keys.rewritten_key(table.columns, tenants) if table.columns.key else None
    before = measure(ch, table)
    expected = Measure(rows=before.rows, keys=measure(ch, table, key=new_key).keys if new_key else 0)

    try:
        ch.execute(ddl.create_like(rel, COPY))
        _set_comment(ch, rel, COPY, MIGRATED)
        ch.execute(ddl.insert_rewritten(rel, COPY, _replacements(table, tenants)))
        copied = measure(ch, table, COPY)
        if not _same(by_keys, before, measure(ch, table)) or not _same(by_keys, expected, copied):
            raise MigrationError(f"{rel}: copy holds {copied}, expected {expected}")
    except Exception as error:
        with contextlib.suppress(Exception):
            ch.execute(ddl.drop(rel, COPY))
        raise MigrationError(f"{rel}: not migrated, the table is unchanged — {error}") from error

    try:
        ch.execute(ddl.exchange(rel, "", COPY))
        live = measure(ch, table)
        if _comments(ch, rel).get("") != MIGRATED or not _same(by_keys, copied, live):
            raise MigrationError(f"{rel}: swapped-in table reads {live}, expected {copied}")
        ch.execute(ddl.rename(rel, COPY, BACKUP))
    except Exception as error:
        raise MigrationError(f"{rel}: failed at the swap — {error}. {_compensate(ch, rel)}") from error
    return live


def _compensate(ch: ClickHouse, rel: Relation) -> str:
    try:
        rollback_relation(ch, rel)
    except Exception as error:  # noqa: BLE001 — reported inside the original failure, which the caller raises
        return f"Putting the original back failed too ({error}); run `rollback` once ClickHouse answers."
    return f"The original is live again; the rewritten copy is kept as {rel}{REJECTED}."


def apply(
    ch: ClickHouse, plan: Plan, dump_dir: Path, *, before_rewrite: list[Callable[[], None]]
) -> list[tuple[str, Measure]]:
    if plan.refusals:
        raise MigrationError("refusing: " + "; ".join(plan.refusals))
    dump(ch, plan, dump_dir)
    for step in before_rewrite:
        step()
    return [(t.table.relation, migrate_table(ch, t, plan.tenants)) for t in plan.changing]


def rollback_relation(ch: ClickHouse, rel: Relation) -> bool:
    """Make the original live and set the rewritten copy aside; False when nothing was rewritten."""
    comments = _comments(ch, rel)
    places = [s for s in ("", COPY, BACKUP) if s in comments]
    rewritten = [s for s in places if comments[s] == MIGRATED]
    originals = [s for s in places if comments[s] != MIGRATED]
    if not rewritten:
        return False
    if len(originals) != 1 or len(rewritten) != 1:
        raise MigrationError(f"{rel}: cannot tell the original apart in {sorted(places)} — leaving it for an operator")
    if REJECTED in comments:
        raise MigrationError(f"{rel}{REJECTED} exists — finalize before rolling back again")

    original = originals[0]
    if original and "" in comments:
        ch.execute(ddl.exchange(rel, "", original))
    elif original:
        ch.execute(ddl.rename(rel, original, ""))

    aside = next(s for s in (COPY, BACKUP) if _comments(ch, rel).get(s) == MIGRATED)
    ch.execute(ddl.rename(rel, aside, REJECTED))
    return True


def _relations_with(ch: ClickHouse, suffixes: tuple[str, ...], marker: str) -> list[Relation]:
    by_suffix = " OR ".join(f"endsWith(name, '{s}')" for s in suffixes)
    rows = ch.rows(
        f"SELECT database, name FROM system.tables WHERE ({by_suffix}) OR comment = '{marker}' ORDER BY database, name"
    )
    found: dict[str, Relation] = {}
    for database, name in rows:
        base = next((name[: -len(s)] for s in WORK if name.endswith(s)), name)
        found.setdefault(f"{database}.{base}", Relation(database, base))
    return list(found.values())


def rollback(ch: ClickHouse) -> list[str]:
    """Put every rewritten table's original back, wherever an interrupted run left it."""
    return [str(rel) for rel in _relations_with(ch, (COPY, BACKUP), MIGRATED) if rollback_relation(ch, rel)]


def finalize(ch: ClickHouse) -> list[str]:
    """Keep what is live, drop every work table, and give a rewritten table back its comment."""
    dropped = []
    for rel in _relations_with(ch, WORK, MIGRATED):
        comments = _comments(ch, rel)
        if "" not in comments:
            raise MigrationError(f"{rel}: no live table — restore or roll back before finalizing")
        if comments[""] == MIGRATED:
            kept = [comments[s] for s in (COPY, BACKUP) if s in comments and comments[s] != MIGRATED]
            _set_comment(ch, rel, "", kept[0] if kept else "")
        for suffix in WORK:
            if suffix in comments:
                ch.execute(ddl.drop(rel, suffix, if_exists=False))
                dropped.append(f"{rel}{suffix}")
    return dropped


def retarget(ddl: str, rel: Relation, suffix: str) -> str:
    """The saved DDL naming the work table; an explicit UUID is dropped, as it is the dumped table's."""
    if not CREATE_TABLE.match(ddl):
        raise MigrationError(f"{rel}: the saved DDL does not start with CREATE TABLE")
    return CREATE_TABLE.sub(f"CREATE TABLE {rel.sql(suffix)}", ddl, count=1)


def _matches_dump(ch: ClickHouse, rel: Relation, suffix: str, entry: dict[str, object]) -> bool:
    """Keys on a replacing table, which merges may collapse after the load; rows elsewhere."""
    if entry["by_keys"]:
        return ch.scalar(f"SELECT uniqExact(unique_key) FROM {rel.sql(suffix)}") == entry["keys"]
    return ch.scalar(f"SELECT count() FROM {rel.sql(suffix)}") == entry["rows"]


def restore_relation(ch: ClickHouse, dump_dir: Path, entry: dict[str, object]) -> None:
    """Load one dumped table into a fresh relation from its saved DDL, check it, and make it live."""
    rel = Relation.parse(str(entry["relation"]))
    comments = _comments(ch, rel)
    if comments.get("") != RESTORED:
        if REJECTED in comments:
            raise MigrationError(f"{rel}{REJECTED} exists — finalize before restoring")
        ch.execute(ddl.drop(rel, RESTORE))
        ch.execute(retarget((dump_dir / f"{rel}.sql").read_text(), rel, RESTORE))
        _set_comment(ch, rel, RESTORE, RESTORED)
        ch.load(rel.sql(RESTORE), dump_dir / f"{rel}.native.gz")
        if not _matches_dump(ch, rel, RESTORE, entry):
            ch.execute(ddl.drop(rel, RESTORE))
            raise MigrationError(f"{rel}: the dump does not load to what the manifest recorded")
        if "" in comments:
            ch.execute(ddl.exchange(rel, "", RESTORE))
        else:
            ch.execute(ddl.rename(rel, RESTORE, ""))

    if _comments(ch, rel).get("") != RESTORED or not _matches_dump(ch, rel, "", entry):
        raise MigrationError(f"{rel}: the live table is not the restored one — run `restore` again")
    if RESTORE in _comments(ch, rel):
        ch.execute(ddl.rename(rel, RESTORE, REJECTED))
    _set_comment(ch, rel, "", str(entry["comment"]))


def restore(ch: ClickHouse, dump_dir: Path) -> list[str]:
    """Every dumped table back as it was dumped; the tables it displaces are kept as rejected."""
    manifest = json.loads((dump_dir / MANIFEST).read_text())
    for entry in manifest["tables"]:
        restore_relation(ch, dump_dir, entry)
    return [entry["relation"] for entry in manifest["tables"]]


def describe(measures: list[tuple[str, Measure]]) -> str:
    return "\n".join(f"{relation}: {json.dumps(asdict(m))}" for relation, m in measures)
