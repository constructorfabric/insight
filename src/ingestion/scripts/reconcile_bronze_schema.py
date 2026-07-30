#!/usr/bin/env python3
"""Reconcile pre-existing bronze tables to the connectors-ddl snapshot.

The snapshot applicator (create-bronze-placeholders.sh) only ever issues
`CREATE TABLE IF NOT EXISTS`, which is a no-op against a table that already
exists. A warm cluster therefore keeps whatever schema its tables had when the
connector last synced: when a connector adds columns, the live table never gains
them, the staging models that read those columns fail with UNKNOWN_IDENTIFIER,
and every downstream silver/gold model is skipped (issue #1991 — an upgrade
froze the git metrics domain because five bitbucket bronze tables predated the
completeness-tracking envelope).

This module closes that gap generically: for every table in the snapshot that
already exists, add the columns the snapshot declares and the live table lacks.
The snapshot is regenerated from real connector output (bootstrap-db +
dump-ddl.sh), so it is the same source of truth that creates fresh tables — a
hand-maintained per-table ALTER list is exactly what drifted before.

Schema introspection is delegated to ClickHouse rather than parsed here: each
snapshot statement is replayed as an empty `<table>__ddl_probe`, and the missing
columns are the set difference against `system.columns`. That way column types
are whatever ClickHouse itself normalises them to, so no type-spelling
comparison can go wrong.

Scope and guarantees:

* bronze_* databases only. silver/insight/staging/identity/person are owned by
  dbt or by the numbered migrations, carry DEFAULT clauses and views, and have
  their own heal semantics in apply-ch-migrations.sh.
* ADD COLUMN only. A column whose type differs from the snapshot is reported and
  left alone: MODIFY COLUMN rewrites data, which is an operator decision. Live
  columns absent from the snapshot are never dropped (legacy or operator columns
  keep their data).
* Idempotent. A reconciled cluster produces an empty diff on the next run.
* Safe against a concurrent sync. `ADD COLUMN IF NOT EXISTS` is a metadata-only
  mutation, and the probes are separate tables — nothing here rewrites or swaps
  a live table.

Adding a Nullable column is metadata-only in ClickHouse, so this stays fast even
on the largest bronze tables.

Used by both callers so the logic cannot diverge:
  * create-bronze-placeholders.sh runs it as a CLI (prod deploy Job).
  * tests/e2e/lib/migration_applier.py imports reconcile() (test rig).
"""

from __future__ import annotations

import logging
import os
import re
import sys
import urllib.request
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass, field
from pathlib import Path

LOG = logging.getLogger("reconcile_bronze_schema")

PROBE_SUFFIX = "__ddl_probe"

# Snapshot statements start with the unquoted `CREATE TABLE IF NOT EXISTS
# db.table` that dump-ddl.sh writes (it rewrites line 1 of SHOW CREATE TABLE).
# Backticks are tolerated in case a future dump quotes identifiers, and table
# names keep their case (bronze_salesforce.OpportunityContactRole).
_CREATE_TABLE_RE = re.compile(
    r"^\s*CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?`?(?P<db>\w+)`?\.`?(?P<table>\w+)`?",
    re.IGNORECASE,
)


@dataclass(frozen=True)
class SnapshotTable:
    """One `CREATE TABLE` statement from the snapshot."""

    database: str
    table: str
    create_sql: str

    @property
    def probe(self) -> str:
        return f"{self.table}{PROBE_SUFFIX}"

    def probe_sql(self) -> str:
        """The same statement retargeted at the probe table.

        Only the matched `CREATE TABLE ... db.table` prefix is rewritten, so the
        column list, ENGINE, ORDER BY and SETTINGS stay byte-identical to the
        snapshot. `IF NOT EXISTS` is dropped: the caller drops the probe first,
        so a surviving probe should surface rather than be silently reused.
        """
        match = _CREATE_TABLE_RE.match(self.create_sql)
        if match is None:  # pragma: no cover — constructed only from a match
            raise ValueError(f"not a CREATE TABLE statement: {self.create_sql[:80]!r}")
        return (
            f"CREATE TABLE `{self.database}`.`{self.probe}`"
            + self.create_sql[match.end() :]
        )


@dataclass
class ReconcileResult:
    added: list[tuple[str, str, str]] = field(default_factory=list)
    type_drift: list[tuple[str, str, str, str]] = field(default_factory=list)
    tables_reconciled: int = 0
    tables_examined: int = 0

    @property
    def columns_added(self) -> int:
        return len(self.added)


def parse_snapshot_tables(sql: str) -> list[SnapshotTable]:
    """Extract every `CREATE TABLE` statement from one snapshot file.

    Statements are separated by a blank line (dump-ddl.sh terminates each with
    `printf ';\\n\\n'`), which is also how the shell applicator splits them.
    Anything that is not a CREATE TABLE — the leading CREATE DATABASE, and the
    views and refreshable MVs in insight.sql — yields no match and is skipped.
    """
    tables: list[SnapshotTable] = []
    for block in re.split(r"\n\s*\n", sql):
        statement = block.strip()
        if not statement:
            continue
        match = _CREATE_TABLE_RE.match(statement)
        if match is None:
            continue
        tables.append(
            SnapshotTable(
                database=match.group("db"),
                table=match.group("table"),
                create_sql=statement,
            )
        )
    return tables


def load_snapshot_tables(ddl_dir: Path) -> list[SnapshotTable]:
    """Every CREATE TABLE across the snapshot, in stable file order."""
    tables: list[SnapshotTable] = []
    for path in sorted(ddl_dir.glob("*.sql")):
        tables.extend(parse_snapshot_tables(path.read_text(encoding="utf-8")))
    return tables


def is_reconcilable(table: SnapshotTable) -> bool:
    """Bronze tables only, and never a probe left over from an aborted run."""
    return table.database.startswith("bronze_") and not table.table.endswith(PROBE_SUFFIX)


def _lit(value: str) -> str:
    """Single-quoted SQL literal (identifiers here are \\w+, so this is belt-only)."""
    return "'" + value.replace("\\", "\\\\").replace("'", "\\'") + "'"


def reconcile(
    tables: Iterable[SnapshotTable],
    *,
    execute: Callable[[str], None],
    fetch_rows: Callable[[str], Sequence[Sequence[str]]],
) -> ReconcileResult:
    """Add snapshot columns missing from each existing bronze table.

    `execute` runs one statement and raises on error; `fetch_rows` runs one
    SELECT and returns its rows as string cells. Injecting both keeps this
    testable and lets the prod CLI and the e2e rig share one implementation.
    """
    result = ReconcileResult()
    candidates = [t for t in tables if is_reconcilable(t)]

    live = _existing_tables(candidates, fetch_rows=fetch_rows)

    for table in candidates:
        if (table.database, table.table) not in live:
            # Absent: the snapshot's own CREATE already made it with the full
            # schema, so there is nothing to reconcile.
            continue
        result.tables_examined += 1
        try:
            _reconcile_one(table, execute=execute, fetch_rows=fetch_rows, result=result)
        finally:
            execute(f"DROP TABLE IF EXISTS `{table.database}`.`{table.probe}`")

    return result


def _existing_tables(
    tables: Sequence[SnapshotTable],
    *,
    fetch_rows: Callable[[str], Sequence[Sequence[str]]],
) -> set[tuple[str, str]]:
    """One round-trip to learn which candidate tables already exist."""
    if not tables:
        return set()
    databases = ", ".join(sorted({_lit(t.database) for t in tables}))
    rows = fetch_rows(
        "SELECT database, name FROM system.tables "
        f"WHERE database IN ({databases})"
    )
    return {(row[0], row[1]) for row in rows if len(row) >= 2}


def _reconcile_one(
    table: SnapshotTable,
    *,
    execute: Callable[[str], None],
    fetch_rows: Callable[[str], Sequence[Sequence[str]]],
    result: ReconcileResult,
) -> None:
    qualified = f"{table.database}.{table.table}"
    execute(f"DROP TABLE IF EXISTS `{table.database}`.`{table.probe}`")
    execute(table.probe_sql())

    db, tbl, probe = _lit(table.database), _lit(table.table), _lit(table.probe)

    missing = fetch_rows(
        "SELECT name, type FROM system.columns "
        f"WHERE database = {db} AND table = {probe} "
        f"AND name NOT IN (SELECT name FROM system.columns WHERE database = {db} AND table = {tbl}) "
        "ORDER BY position"
    )
    for row in missing:
        name, ch_type = row[0], row[1]
        execute(
            f"ALTER TABLE `{table.database}`.`{table.table}` "
            f"ADD COLUMN IF NOT EXISTS `{name}` {ch_type}"
        )
        result.added.append((qualified, name, ch_type))
        LOG.info("  + %s.%s %s", qualified, name, ch_type)

    # Report-only: a differing type means the live table would need a data
    # rewrite (MODIFY COLUMN), which is never done unattended.
    drift = fetch_rows(
        "SELECT s.name, s.type, l.type FROM "
        f"(SELECT name, type FROM system.columns WHERE database = {db} AND table = {probe}) AS s "
        "INNER JOIN "
        f"(SELECT name, type FROM system.columns WHERE database = {db} AND table = {tbl}) AS l "
        "USING (name) WHERE s.type != l.type ORDER BY s.name"
    )
    for row in drift:
        name, snapshot_type, live_type = row[0], row[1], row[2]
        result.type_drift.append((qualified, name, snapshot_type, live_type))
        LOG.warning(
            "  ! %s.%s type differs — snapshot=%s live=%s (left unchanged)",
            qualified,
            name,
            snapshot_type,
            live_type,
        )

    if missing:
        result.tables_reconciled += 1


def _http_client() -> tuple[Callable[[str], None], Callable[[str], Sequence[Sequence[str]]]]:
    """Executors over the ClickHouse HTTP interface, mirroring lib/ch-exec.sh.

    ClickHouse is always external to the release, so HTTP is the only path. The
    password travels in a header (not argv) exactly as ch-exec.sh does.
    """
    url = os.environ.get("CLICKHOUSE_URL")
    user = os.environ.get("CLICKHOUSE_USER")
    password = os.environ.get("CLICKHOUSE_PASSWORD")
    missing = [
        name
        for name, value in (
            ("CLICKHOUSE_URL", url),
            ("CLICKHOUSE_USER", user),
            ("CLICKHOUSE_PASSWORD", password),
        )
        if not value
    ]
    if missing:
        raise SystemExit(f"{', '.join(missing)} must be set")

    endpoint = url.rstrip("/") + "/"

    def _post(sql: str) -> str:
        request = urllib.request.Request(  # noqa: S310 — fixed http(s) endpoint from config
            endpoint,
            data=sql.encode("utf-8"),
            headers={"X-ClickHouse-User": user, "X-ClickHouse-Key": password},
            method="POST",
        )
        with urllib.request.urlopen(request) as response:  # noqa: S310
            return response.read().decode("utf-8")

    def execute(sql: str) -> None:
        _post(sql)

    def fetch_rows(sql: str) -> list[list[str]]:
        body = _post(sql)
        return [line.split("\t") for line in body.splitlines() if line]

    return execute, fetch_rows


def main(argv: Sequence[str] | None = None) -> int:
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    args = list(argv if argv is not None else sys.argv[1:])
    ddl_dir = Path(args[0]) if args else Path(__file__).resolve().parent / "connectors-ddl"
    if not ddl_dir.is_dir():
        raise SystemExit(f"DDL snapshot directory not found: {ddl_dir}")

    tables = load_snapshot_tables(ddl_dir)
    execute, fetch_rows = _http_client()
    result = reconcile(tables, execute=execute, fetch_rows=fetch_rows)

    LOG.info(
        "  reconciled %d column(s) across %d of %d existing bronze table(s)",
        result.columns_added,
        result.tables_reconciled,
        result.tables_examined,
    )
    if result.type_drift:
        LOG.warning(
            "  %d column(s) differ in type from the snapshot and were left unchanged",
            len(result.type_drift),
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
