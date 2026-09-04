"""Rig for the Jira field-history transformation tests.

One ClickHouse, one dbt project, real models. A test seeds the three bronze
tables the chain reads, builds it, and reads the journal back:

    jira_fields + jira_issue + jira_issue_history
        -> dbt build --select tag:jira,tag:staging
        -> staging.jira__field_history_derived

Nothing is stubbed. Bronze is created from `scripts/connectors-ddl/jira.sql` —
the snapshot the connectors-ddl gate keeps byte-identical to what the real
connectors produce — so the tables have production's engines and column types,
including the plain MergeTree that `jira__bronze_promoted` then promotes.

Deliberately independent of `tests/e2e`: that rig boots MariaDB, Keycloak stubs
and the analytics binary to assert an HTTP response, none of which says anything
about these transformations. This lane needs a warehouse and dbt.

Connection comes from the environment with no defaults — a suite that silently
falls back to localhost either tests nothing or writes into somebody's
warehouse. See README.md for the two commands that set it up.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import tempfile
from pathlib import Path
from typing import Any

import clickhouse_connect
import pytest
from helpers import SOURCE_ID

# tests/jira/transform -> tests/jira -> tests -> dbt
DBT_DIR = Path(__file__).resolve().parents[3]
INGESTION_DIR = DBT_DIR.parent
JIRA_BRONZE_DDL = INGESTION_DIR / "scripts" / "connectors-ddl" / "jira.sql"

# What the prod staging step selects. Building it is its own assertion — that
# the field-history models coexist with every other Jira staging model — and
# `test_invariants` makes it once.
PROD_SELECTOR = "tag:jira,tag:staging"

# What a scenario actually needs: the journal, the side table, and their
# ancestors. Comment, worklog, availability and project-visibility models are in
# the prod selector but read none of the three bronze tables these tests seed,
# so building them per test costs about thirty seconds and proves nothing.
FIELD_HISTORY_SELECTOR = "+jira__field_history_derived +jira__task_field_text +jira__task_field_unclassified"

JOURNAL = "staging.jira__field_history_derived"

BRONZE_TABLES = ("bronze_jira.jira_fields", "bronze_jira.jira_issue", "bronze_jira.jira_issue_history")


def _env(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(
            f"{name} is not set. This suite needs a ClickHouse to build against; see tests/jira/transform/README.md."
        )
    return value


class Warehouse:
    """The ClickHouse under test, plus the dbt invocation that targets it."""

    def __init__(self, profiles_dir: Path) -> None:
        self.host = _env("CLICKHOUSE_HOST")
        self.port = int(_env("CLICKHOUSE_HTTP_PORT"))
        self.user = _env("CLICKHOUSE_USER")
        self.password = _env("CLICKHOUSE_PASSWORD")
        self.profiles_dir = profiles_dir

    def client(self, database: str = "default"):
        return clickhouse_connect.get_client(
            host=self.host, port=self.port, username=self.user, password=self.password, database=database
        )

    def execute(self, sql: str) -> None:
        with self.client() as c:
            c.command(sql)

    def rows(self, sql: str) -> list[dict[str, Any]]:
        with self.client() as c:
            result = c.query(sql)
            return [dict(zip(result.column_names, row)) for row in result.result_rows]

    def insert(self, table: str, records: list[dict[str, Any]]) -> None:
        """Insert dict rows by name, so a fixture never has to order columns."""
        if not records:
            return
        columns = sorted({k for r in records for k in r})
        payload = "\n".join(json.dumps({c: r.get(c) for c in columns}, default=str) for r in records)
        with self.client() as c:
            c.raw_insert(table, column_names=columns, insert_block=payload, fmt="JSONEachRow")

    def dbt_status(self, *args: str) -> tuple[int, str]:
        """Run dbt and hand back its exit code and output, judging nothing."""
        proc = subprocess.run(
            ["dbt", *args, "--profiles-dir", str(self.profiles_dir)], cwd=DBT_DIR, capture_output=True, text=True
        )
        return proc.returncode, f"{proc.stdout[-6000:]}\n{proc.stderr[-2000:]}"

    def dbt(self, *args: str) -> None:
        """Run dbt and fail the test with its output when it errors."""
        code, output = self.dbt_status(*args)
        if code != 0:
            pytest.fail(f"dbt {' '.join(args)} failed (exit {code}):\n{output}", pytrace=False)

    def build(self, selector: str = FIELD_HISTORY_SELECTOR) -> None:
        # `run`, not `build`: `build` interleaves the singular tests, so a
        # scenario written to make an invariant fail — and there is one, because
        # the failure is the point — would look like a broken model instead.
        # Invariants are asserted explicitly, per scenario, below.
        self.dbt("run", "--select", *selector.split())


def _apply_sql_file(warehouse: Warehouse, path: Path) -> None:
    """Apply a multi-statement .sql file one statement at a time.

    The HTTP endpoint takes one statement per request. Splitting on `;` is safe
    for this file: it is generated DDL with no semicolon inside any literal.
    """
    for statement in re.split(r";\s*\n", path.read_text()):
        if statement.strip():
            warehouse.execute(statement)


@pytest.fixture(scope="session")
def warehouse() -> Warehouse:
    """Session setup: databases, bronze DDL, a dbt profile pointing at them."""
    with tempfile.TemporaryDirectory() as tmp:
        profiles_dir = Path(tmp)
        wh = Warehouse(profiles_dir)
        # `password` is inlined rather than read through env_var: this profile is
        # written to a private temp dir for one session and never committed.
        profiles_dir.joinpath("profiles.yml").write_text(
            "ingestion:\n"
            "  target: test\n"
            "  outputs:\n"
            "    test:\n"
            "      type: clickhouse\n"
            f"      host: {wh.host}\n"
            f"      port: {wh.port}\n"
            f"      user: {wh.user}\n"
            f"      password: {wh.password}\n"
            "      schema: silver\n"
            "      secure: false\n"
            "      query_limit: 0\n"
            "      connect_timeout: 30\n"
            "      send_receive_timeout: 600\n"
            "      settings:\n"
            # Parity with prod and the bootstrap profile: dbt-clickhouse does not
            # push a model-level setting into the SELECT plan, so it lives here.
            "        allow_experimental_correlated_subqueries: 1\n"
        )
        for database in ("staging", "silver", "config", "insight"):
            wh.execute(f"CREATE DATABASE IF NOT EXISTS {database}")
        _apply_sql_file(wh, JIRA_BRONZE_DDL)
        yield wh


class Scenario:
    """One test's data: seed, build, read the journal back."""

    def __init__(self, warehouse: Warehouse) -> None:
        self.warehouse = warehouse

    def seed(
        self, *, fields: list[dict[str, Any]], issues: list[dict[str, Any]], events: list[dict[str, Any]] | None = None
    ) -> None:
        self.warehouse.insert("bronze_jira.jira_fields", fields)
        self.warehouse.insert("bronze_jira.jira_issue", issues)
        self.warehouse.insert("bronze_jira.jira_issue_history", events or [])

    def build(self, selector: str = FIELD_HISTORY_SELECTOR) -> None:
        self.warehouse.build(selector)

    def journal(self, *, issue: str | None = None, field: str | None = None) -> list[dict[str, Any]]:
        """The journal rows, ordered the way a reader reconstructs history.

        FINAL because the model is a ReplacingMergeTree and unmerged parts would
        otherwise read as duplicates.
        """
        where = [f"insight_source_id = '{SOURCE_ID}'"]
        if issue:
            where.append(f"id_readable = '{issue}'")
        if field:
            where.append(f"field_id = '{field}'")
        return self.warehouse.rows(
            "SELECT field_id, event_kind, event_id, toString(event_at) AS event_at, _seq,"
            "       field_cardinality, delta_action, value_ids, value_displays, value_id_type,"
            "       author_id"
            f" FROM {JOURNAL} FINAL"
            f" WHERE {' AND '.join(where)}"
            # The reading order, matching the round-trip invariant: the kind
            # first (an initial row is the state at creation, so it precedes any
            # event of the same instant), then `_seq` among the initial rows,
            # then the event id numerically because '101' sorts before '99'.
            " ORDER BY field_id, event_at,"
            "          multiIf(event_kind = 'synthetic_initial', 0,"
            "                  event_kind = 'changelog', 1, 2),"
            "          _seq, toUInt64OrZero(event_id), event_id"
        )

    def invariants_hold(self, select: str = "tests/jira") -> bool:
        """Do the dbt singular tests pass over what this scenario produced?

        Returned rather than asserted: most scenarios require them to hold, and
        at least one requires the round trip NOT to — a value the source changed
        without recording it cannot be reconciled, and a test that hid that
        would be worse than one that fails.

        A WARNING counts as not holding. Two tests are `severity: warn` so the
        nightly run reports an unrepairable source condition without failing on
        it (§3.4, §3.5) — but here the inputs are controlled, so anything those
        tests find IS a pipeline fault. Reading only the exit code would make
        every scenario that depends on them silently vacuous.
        """
        code, output = self.warehouse.dbt_status("test", "--select", select)
        if code != 0:
            return False
        # dbt's summary line: "Done. PASS=n WARN=n ERROR=n SKIP=n ..."
        warns = re.search(r"\bWARN=(\d+)", output)
        return not (warns and int(warns.group(1)) > 0)

    def round_trip_holds(self) -> bool:
        """The oracle on its own: does replaying each field land on the value
        the issue holds?"""
        return self.invariants_hold("assert_jira_field_history_round_trip")

    def text_rows(self) -> list[dict[str, Any]]:
        """The long-text side table, addressed by content hash (§8)."""
        return self.warehouse.rows(
            "SELECT text_id, content_form, content"
            " FROM staging.jira__task_field_text FINAL"
            " ORDER BY content_form, content"
        )

    def states(self, field: str, *, issue: str | None = None) -> list[list[str]]:
        """Just the value_ids of one field's rows, in order — the common assert."""
        return [row["value_ids"] for row in self.journal(issue=issue, field=field)]

    def changelog_states(self, field: str, *, issue: str | None = None) -> list[list[str]]:
        """The same, restricted to rows an event produced.

        Use it when the scenario is about how an ITEM is read: an issue whose
        JSON does not carry the key also gets a withdrawal row (§3.6), and that
        row is the last one, so `states(...)[-1]` would be its empty value
        rather than the state the event described.
        """
        return [row["value_ids"] for row in self.journal(issue=issue, field=field) if row["event_kind"] == "changelog"]


@pytest.fixture
def scenario(warehouse: Warehouse) -> Scenario:
    """A clean warehouse per test.

    Truncating bronze rather than scoping every assertion to a source id keeps
    the expectations exact: a test says which rows the journal holds, not which
    rows it holds among others. The staging models are `table`-materialized, so
    the next build rewrites them from the bronze this test seeded.
    """
    for table in BRONZE_TABLES:
        warehouse.execute(f"TRUNCATE TABLE IF EXISTS {table}")
    return Scenario(warehouse)
