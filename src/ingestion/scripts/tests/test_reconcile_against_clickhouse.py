"""Opt-in integration test: reconcile a real warm ClickHouse (issue #1991).

The unit tests pin the algorithm against a stand-in; this one pins it against
ClickHouse itself — the probe replay, the `system.columns` diff, and the ALTERs
all run for real, so it also proves the generated SQL is valid for the pinned
server version.

Skipped unless a server is offered, so CI and local `pytest` stay dependency-free:

    docker run -d --rm --name ch -p 38200:8123 \\
        -e CLICKHOUSE_PASSWORD=x clickhouse/clickhouse-server:25.7.5.34
    RECONCILE_TEST_CH_URL=http://localhost:38200 \\
    RECONCILE_TEST_CH_PASSWORD=x .venv/bin/python -m pytest tests -q

It reproduces the reported failure shape rather than asserting on a fixture: for
each table the issue names, the committed snapshot DDL is replayed with the
post-upgrade columns withheld, which is exactly what an upgraded install holds
(the table exists, so `CREATE TABLE IF NOT EXISTS` never widens it).
"""

from __future__ import annotations

import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import reconcile_bronze_schema as rbs  # noqa: E402

CH_URL = os.environ.get("RECONCILE_TEST_CH_URL")
CH_USER = os.environ.get("RECONCILE_TEST_CH_USER", "default")
CH_PASSWORD = os.environ.get("RECONCILE_TEST_CH_PASSWORD", "")

pytestmark = pytest.mark.skipif(
    not CH_URL,
    reason="set RECONCILE_TEST_CH_URL (and RECONCILE_TEST_CH_PASSWORD) to run against a live ClickHouse",
)

DDL_DIR = Path(__file__).resolve().parent.parent / "connectors-ddl"

# The tables issue #1991 lists, with the columns their staging models could not
# find. bronze_bitbucket_cloud gained the completeness-tracking envelope; the
# outline user snapshot gained its own set.
ENVELOPE = {
    "record_type", "generation_id", "bucket_id", "snapshot_item_count",
    "snapshot_available", "entity_key", "repository_uuid", "workspace_uuid",
    "data_source", "collected_at",
}
OUTLINE_COLUMNS = {
    "name", "role", "is_suspended", "data_source", "last_active_at",
    "created_at", "collected_at",
}
WITHHELD = {
    ("bronze_bitbucket_cloud", table): ENVELOPE
    for table in (
        "repositories", "branches", "commits", "file_changes",
        "pull_requests", "pull_request_comments", "pull_request_commits",
    )
}
WITHHELD[("bronze_outline", "wiki_users")] = OUTLINE_COLUMNS

_COLUMN = re.compile(r"^\s*`(?P<name>[^`]+)`\s+.+?,?\s*$")


def post(sql: str) -> str:
    request = urllib.request.Request(  # noqa: S310 — operator-supplied test endpoint
        CH_URL.rstrip("/") + "/",
        data=sql.encode("utf-8"),
        headers={"X-ClickHouse-User": CH_USER, "X-ClickHouse-Key": CH_PASSWORD},
    )
    try:
        with urllib.request.urlopen(request) as response:  # noqa: S310
            return response.read().decode("utf-8")
    except urllib.error.HTTPError as exc:  # surface ClickHouse's own message
        raise AssertionError(f"{exc.code}: {exc.read().decode('utf-8', 'replace')}\n{sql[:200]}") from exc


def rows(sql: str) -> list[list[str]]:
    return [line.split("\t") for line in post(sql).splitlines() if line]


def withhold_columns(create_sql: str, drop: set[str]) -> str:
    """The snapshot statement minus `drop`, i.e. the table's pre-upgrade shape.

    Columns named by ENGINE/ORDER BY/SETTINGS are kept regardless, so the
    reduced statement stays valid.
    """
    head, rest = create_sql.split("(\n", 1)
    close = rest.index("\n)")
    body, tail = rest[:close], rest[close + 2 :]
    protected = set(re.findall(r"\b(\w+)\b", tail))
    kept = [
        line.rstrip().rstrip(",")
        for line in body.splitlines()
        if line.strip()
        and not ((m := _COLUMN.match(line)) and m.group("name") in drop and m.group("name") not in protected)
    ]
    return (head + "(\n" + ",\n".join(kept) + "\n)" + tail).replace("IF NOT EXISTS ", "").rstrip(";")


@pytest.fixture
def warm_cluster():
    """A cluster whose tables predate the columns their staging models read."""
    snapshot = {(t.database, t.table): t for t in rbs.load_snapshot_tables(DDL_DIR)}
    created = []
    for key, drop in WITHHELD.items():
        table = snapshot.get(key)
        assert table is not None, f"{key} missing from the snapshot — update this test"
        database, name = key
        post(f"CREATE DATABASE IF NOT EXISTS `{database}`")
        post(f"DROP TABLE IF EXISTS `{database}`.`{name}`")
        post(withhold_columns(table.create_sql, drop))
        first = rows(
            f"SELECT name FROM system.columns WHERE database='{database}' AND table='{name}' ORDER BY position"
        )[0][0]
        post(f"INSERT INTO `{database}`.`{name}` (`{first}`) VALUES ('legacy-row')")
        created.append(key)
    yield snapshot
    for database, name in created:
        post(f"DROP TABLE IF EXISTS `{database}`.`{name}`")


def reconcile_all(snapshot):
    return rbs.reconcile(snapshot.values(), execute=post, fetch_rows=rows)


def columns_of(database: str, table: str) -> set[str]:
    return {
        row[0]
        for row in rows(f"SELECT name FROM system.columns WHERE database='{database}' AND table='{table}'")
    }


def test_warm_cluster_starts_without_the_columns(warm_cluster):
    """Guards the fixture: without this the healing assertions prove nothing."""
    for (database, table), withheld in WITHHELD.items():
        live = columns_of(database, table)
        assert withheld - live, f"{database}.{table} already has {withheld} — fixture is not reproducing #1991"


def test_reconcile_adds_every_missing_column(warm_cluster):
    result = reconcile_all(warm_cluster)

    assert result.columns_added > 0
    for (database, table), withheld in WITHHELD.items():
        live = columns_of(database, table)
        missing = withheld - live
        assert not missing, f"{database}.{table} still missing {sorted(missing)}"


def test_reconcile_preserves_existing_rows(warm_cluster):
    reconcile_all(warm_cluster)

    for database, table in WITHHELD:
        count = rows(f"SELECT count() FROM `{database}`.`{table}`")[0][0]
        assert count == "1", f"{database}.{table} lost its row"


def test_reconcile_is_idempotent(warm_cluster):
    reconcile_all(warm_cluster)

    second = reconcile_all(warm_cluster)

    assert second.columns_added == 0
    assert second.tables_reconciled == 0


def test_reconcile_leaves_no_probe_tables(warm_cluster):
    reconcile_all(warm_cluster)

    leftovers = rows(
        "SELECT database, name FROM system.tables "
        f"WHERE name LIKE '%{rbs.PROBE_SUFFIX}' FORMAT TSV"
    )
    assert leftovers == []


# ---------------------------------------------------------------------------
# Whole-snapshot sweep
# ---------------------------------------------------------------------------
# The tests above cover the tables issue #1991 named. These cover EVERY bronze
# table in the snapshot — 172 across 25 connector databases — each stripped to
# the bare minimum ClickHouse will accept, which is far more drift than a real
# upgrade produces. The assertion is the strong one: after reconcile, a stripped
# table's columns match what a fresh install of that same snapshot statement
# creates, compared by ClickHouse itself rather than by parsing DDL text.

REFERENCE_SUFFIX = "__ddl_reference"


def _split_columns(create_sql: str) -> tuple[str, str, str]:
    head, rest = create_sql.split("(\n", 1)
    close = rest.index("\n)")
    return head, rest[:close], rest[close + 2 :]


def strip_to_minimum(create_sql: str) -> tuple[str, set[str]]:
    """Withhold every column the ENGINE/ORDER BY/SETTINGS tail does not require.

    Whatever the tail names must stay or the CREATE is invalid; at least one
    column is always kept. Returns the reduced statement and the withheld names.
    """
    head, body, tail = _split_columns(create_sql)
    required = set(re.findall(r"\b(\w+)\b", tail))
    kept, withheld = [], set()
    for line in body.splitlines():
        match = _COLUMN.match(line)
        if not match:
            continue
        if match.group("name") in required:
            kept.append(line)
        else:
            withheld.add(match.group("name"))
    if not kept:  # ORDER BY tuple() protects nothing — keep the first column
        first = next(line for line in body.splitlines() if _COLUMN.match(line))
        kept.append(first)
        withheld.discard(_COLUMN.match(first).group("name"))
    reduced = head + "(\n" + ",\n".join(line.rstrip().rstrip(",") for line in kept) + "\n)" + tail
    return reduced.replace("IF NOT EXISTS ", "").rstrip(";"), withheld


def retarget(create_sql: str, database: str, table: str) -> str:
    """The snapshot statement pointed at a different table name."""
    match = rbs._CREATE_TABLE_RE.match(create_sql)
    return f"CREATE TABLE `{database}`.`{table}`" + create_sql[match.end() :].rstrip(";")


def columns_with_types(database: str, table: str) -> set[tuple[str, str]]:
    return {
        (row[0], row[1])
        for row in rows(
            f"SELECT name, type FROM system.columns WHERE database='{database}' AND table='{table}' FORMAT TSV"
        )
    }


@pytest.fixture(scope="module")
def stripped_snapshot():
    """Every bronze table in the snapshot, created stripped to its minimum."""
    tables = rbs.load_snapshot_tables(DDL_DIR)
    bronze = [t for t in tables if rbs.is_reconcilable(t)]
    assert len(bronze) > 100, f"expected the full snapshot, got {len(bronze)} bronze tables"

    withheld_total = 0
    for table in bronze:
        reduced, withheld = strip_to_minimum(table.create_sql)
        post(f"CREATE DATABASE IF NOT EXISTS `{table.database}`")
        post(f"DROP TABLE IF EXISTS `{table.database}`.`{table.table}`")
        post(reduced)
        first = rows(
            f"SELECT name FROM system.columns WHERE database='{table.database}' "
            f"AND table='{table.table}' ORDER BY position"
        )[0][0]
        post(f"INSERT INTO `{table.database}`.`{table.table}` (`{first}`) VALUES (DEFAULT)")
        withheld_total += len(withheld)

    yield {"tables": tables, "bronze": bronze, "withheld_total": withheld_total}

    for table in bronze:
        post(f"DROP TABLE IF EXISTS `{table.database}`.`{table.table}`")
        post(f"DROP TABLE IF EXISTS `{table.database}`.`{table.table}{REFERENCE_SUFFIX}`")


def test_sweep_actually_withholds_columns(stripped_snapshot):
    """Guards the sweep fixture — otherwise the healing assertion is vacuous."""
    assert stripped_snapshot["withheld_total"] > 1000, stripped_snapshot["withheld_total"]


def test_every_bronze_table_matches_a_fresh_install(stripped_snapshot):
    """The core generality claim, across all 25 connector databases."""
    result = rbs.reconcile(stripped_snapshot["tables"], execute=post, fetch_rows=rows)
    assert result.columns_added == stripped_snapshot["withheld_total"]

    mismatched = []
    for table in stripped_snapshot["bronze"]:
        reference = f"{table.table}{REFERENCE_SUFFIX}"
        post(f"DROP TABLE IF EXISTS `{table.database}`.`{reference}`")
        post(retarget(table.create_sql, table.database, reference))
        expected = columns_with_types(table.database, reference)
        actual = columns_with_types(table.database, table.table)
        if actual != expected:
            mismatched.append(
                f"{table.database}.{table.table}: missing={sorted(expected - actual)} extra={sorted(actual - expected)}"
            )
    assert not mismatched, "\n".join(mismatched)


def test_sweep_preserves_every_row(stripped_snapshot):
    rbs.reconcile(stripped_snapshot["tables"], execute=post, fetch_rows=rows)

    empty = [
        f"{t.database}.{t.table}"
        for t in stripped_snapshot["bronze"]
        if rows(f"SELECT count() FROM `{t.database}`.`{t.table}`")[0][0] != "1"
    ]
    assert not empty, f"rows lost in: {empty}"


def test_sweep_is_idempotent(stripped_snapshot):
    rbs.reconcile(stripped_snapshot["tables"], execute=post, fetch_rows=rows)

    second = rbs.reconcile(stripped_snapshot["tables"], execute=post, fetch_rows=rows)

    assert second.columns_added == 0
    assert second.type_drift == []


def test_sweep_never_touches_non_bronze_tables(stripped_snapshot):
    """silver/insight/identity/person/staging are owned by dbt and the migrations."""
    non_bronze = [t for t in stripped_snapshot["tables"] if not rbs.is_reconcilable(t)]
    assert non_bronze, "snapshot should contain non-bronze tables"
    created = []
    try:
        for table in non_bronze:
            reduced, withheld = strip_to_minimum(table.create_sql)
            if not withheld:
                continue
            post(f"CREATE DATABASE IF NOT EXISTS `{table.database}`")
            post(f"DROP TABLE IF EXISTS `{table.database}`.`{table.table}`")
            post(reduced)
            created.append((table, withheld))

        rbs.reconcile(stripped_snapshot["tables"], execute=post, fetch_rows=rows)

        widened = [
            f"{t.database}.{t.table}"
            for t, withheld in created
            if withheld & {name for name, _ in columns_with_types(t.database, t.table)}
        ]
        assert not widened, f"non-bronze tables were modified: {widened}"
    finally:
        for table, _ in created:
            post(f"DROP TABLE IF EXISTS `{table.database}`.`{table.table}`")


def test_sweep_leaves_no_scratch_tables(stripped_snapshot):
    rbs.reconcile(stripped_snapshot["tables"], execute=post, fetch_rows=rows)

    leftovers = rows(
        f"SELECT database, name FROM system.tables WHERE name LIKE '%{rbs.PROBE_SUFFIX}' FORMAT TSV"
    )
    assert leftovers == []
