"""Rig for the Jira field-history transformation tests.

One ClickHouse, one dbt project, real models. A test seeds the three bronze
tables the chain reads, builds it, and reads the journal back:

    jira_fields + jira_issue + jira_issue_history
        -> dbt build --select tag:jira,tag:staging
        -> staging.jira__field_history_derived

Nothing is stubbed. Bronze is created from `scripts/connectors-ddl/jira.sql` —
the snapshot the connectors-ddl gate keeps byte-identical to what the real
connectors produce — so the tables have production's engines and column types,
including the ReplacingMergeTree shape the Airbyte destination creates.

Deliberately independent of `tests/e2e`: that rig boots MariaDB, Keycloak stubs
and the analytics binary to assert an HTTP response, none of which says anything
about these transformations. This lane needs a warehouse and dbt.

Connection comes from the environment with no defaults — a suite that silently
falls back to localhost either tests nothing or writes into somebody's
warehouse. See README.md for the two commands that set it up.
"""

from __future__ import annotations

import functools
import json
import os
import re
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import clickhouse_connect
import pytest
from dbt.cli.main import dbtRunner
from helpers import SOURCE_ID

# tests/jira/transform -> tests/jira -> tests -> dbt
DBT_DIR = Path(__file__).resolve().parents[3]
INGESTION_DIR = DBT_DIR.parent
JIRA_BRONZE_DDL = INGESTION_DIR / "scripts" / "connectors-ddl" / "jira.sql"
# The class tables gold reads. `test_title_role.py` seeds them directly, and
# the union models that normally create them need every source's staging arm.
SILVER_DDL = INGESTION_DIR / "scripts" / "connectors-ddl" / "silver.sql"

# What the prod staging step selects. Building it is its own assertion — that
# the field-history models coexist with every other Jira staging model — and
# `test_invariants` makes it once.
PROD_SELECTOR = "tag:jira,tag:staging"

# What a scenario actually needs: the journal, the side table, and their
# ancestors. Comment, worklog, availability and project-visibility models are in
# the prod selector but read none of the three bronze tables these tests seed,
# so building them per test costs about thirty seconds and proves nothing.
FIELD_HISTORY_SELECTOR = "+jira__field_history_derived +jira__task_field_text +jira__task_field_unclassified"


def _env(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(
            f"{name} is not set. This suite needs a ClickHouse to build against; see tests/jira/transform/README.md."
        )
    return value


@dataclass(frozen=True)
class Invocation:
    """What one dbt call did: whether it succeeded, what each node reported,
    and enough of the log to read a failure from."""

    success: bool
    statuses: tuple[str, ...]
    log: str


class Dbt:
    """The dbt project, invoked in the test process rather than as a command.

    A subprocess pays for an interpreter start on every call — about nine
    seconds against three of actual work, and a scenario makes two calls.
    Invoking in-process keeps the interpreter and leaves dbt's own parse, which
    reads the partial-parse cache and costs a couple of seconds.

    INVARIANT: every call gets a freshly parsed manifest. Reusing one is
    tempting and about a second faster, but dbt mutates it while compiling —
    a node built once is marked compiled, so a second run of a model with an
    ephemeral parent no longer gets the parent's CTE injected and references a
    name that was never defined.
    """

    def __init__(self, profiles_dir: Path) -> None:
        self._base = ["--project-dir", str(DBT_DIR), "--profiles-dir", str(profiles_dir), "--quiet"]
        self._log: list[str] = []
        parsed = self._invoke("parse")
        if not parsed.success:
            raise RuntimeError(f"dbt could not parse the project:\n{parsed.log}")

    def _invoke(self, *args: str) -> Invocation:
        self._log.clear()
        runner = dbtRunner(callbacks=[lambda event: self._log.append(event.info.msg)])
        result = runner.invoke([*args, *self._base])
        statuses = tuple(str(node.status) for node in getattr(result.result, "results", ()))
        log = "\n".join(self._log)
        if result.exception is not None:
            log = f"{log}\n{result.exception}"
        return Invocation(success=result.success, statuses=statuses, log=log[-8000:])

    def invoke(self, *args: str) -> Invocation:
        return self._invoke(*args)


class Warehouse:
    """The ClickHouse under test, plus the dbt invocation that targets it."""

    def __init__(self, profiles_dir: Path) -> None:
        self.host = _env("CLICKHOUSE_HOST")
        self.port = int(_env("CLICKHOUSE_HTTP_PORT"))
        self.user = _env("CLICKHOUSE_USER")
        self.password = _env("CLICKHOUSE_PASSWORD")
        self.profiles_dir = profiles_dir
        # Bumped whenever bronze is truncated, so a shared build knows its rows
        # were taken out from under it.
        self.generation = 0

    def client(self, database: str = "default"):
        return clickhouse_connect.get_client(
            host=self.host, port=self.port, username=self.user, password=self.password, database=database
        )

    def execute(self, sql: str, parameters: dict[str, Any] | None = None) -> None:
        """Run a statement. Values go through `parameters`, never through the
        string — ClickHouse binds them server-side, so nothing in a fixture can
        end up parsed as SQL."""
        with self.client() as c:
            c.command(sql, parameters=parameters)

    def rows(self, sql: str, parameters: dict[str, Any] | None = None) -> list[dict[str, Any]]:
        with self.client() as c:
            result = c.query(sql, parameters=parameters)
            return [dict(zip(result.column_names, row)) for row in result.result_rows]

    def insert(self, table: str, records: list[dict[str, Any]]) -> None:
        """Insert dict rows by name, so a fixture never has to order columns."""
        if not records:
            return
        columns = sorted({k for r in records for k in r})
        payload = "\n".join(json.dumps({c: r.get(c) for c in columns}, default=str) for r in records)
        with self.client() as c:
            c.raw_insert(table, column_names=columns, insert_block=payload, fmt="JSONEachRow")

    @functools.cached_property
    def dbt_project(self) -> Dbt:
        """Parsed on first use, then reused for the rest of the session."""
        return Dbt(self.profiles_dir)

    def dbt_status(self, *args: str) -> Invocation:
        """Run dbt and hand back what it did, judging nothing."""
        return self.dbt_project.invoke(*args)

    def dbt(self, *args: str) -> None:
        """Run dbt and fail the test with its output when it errors."""
        invocation = self.dbt_status(*args)
        if not invocation.success:
            pytest.fail(f"dbt {' '.join(args)} failed:\n{invocation.log}", pytrace=False)

    def build(self, selector: str = FIELD_HISTORY_SELECTOR) -> None:
        # `run`, not `build`: `build` interleaves the singular tests, so a
        # scenario written to make an invariant fail — and there is one, because
        # the failure is the point — would look like a broken model instead.
        # Invariants are asserted explicitly, per scenario, below.
        #
        # `--full-refresh` because one model is incremental and keeps state
        # across runs on purpose (`jira__catalogue_first_seen`): every scenario
        # seeds its own bronze and reads its own answer. The one test that is
        # ABOUT the persistence runs that model again without the flag.
        self.dbt("run", "--select", *selector.split(), "--full-refresh")


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
    """Session setup: databases, bronze and silver DDL, a dbt profile pointing at them."""
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
        wh.execute("CREATE DATABASE IF NOT EXISTS staging")
        wh.execute("CREATE DATABASE IF NOT EXISTS silver")
        wh.execute("CREATE DATABASE IF NOT EXISTS config")
        wh.execute("CREATE DATABASE IF NOT EXISTS insight")
        _apply_sql_file(wh, JIRA_BRONZE_DDL)
        _apply_sql_file(wh, SILVER_DDL)
        # Parse here rather than inside whichever test runs first, so a broken
        # project reads as a setup error and costs one test no seconds.
        _ = wh.dbt_project
        yield wh


@dataclass(frozen=True)
class Case:
    """The bronze one scenario is about, declared beside the test that reads it."""

    fields: list[dict[str, Any]]
    issues: list[dict[str, Any]]
    events: list[dict[str, Any]]


def case(
    *, fields: list[dict[str, Any]], issues: list[dict[str, Any]], events: list[dict[str, Any]] | None = None
) -> Any:
    """Declare a test's bronze so its module can be seeded and built in one go.

    A build costs about fifteen seconds whatever it builds — nearly all of it
    ClickHouse analysing a query it then analyses again to insert — so the
    suite's runtime is the NUMBER of builds, not their content. A decorated
    test's rows go in with the rest of its module's, under a source id of its
    own, and every scenario in the module reads its own answer out of one build.

    A test the batch cannot hold — one that builds twice, expects a build to
    fail, or rewrites bronze half way through — simply carries no `case` and
    gets an exclusive warehouse, as every test used to.
    """

    def declare(test: Any) -> Any:
        test.case = Case(fields=fields, issues=issues, events=events or [])
        return test

    return declare


class Scenario:
    """One test's data: seed, build, read the journal back.

    `source` is the isolation: every model in the chain carries
    `insight_source_id` through its joins and windows, so scenarios sharing a
    build never see each other's rows.
    """

    def __init__(self, warehouse: Warehouse, source: str = SOURCE_ID) -> None:
        self.warehouse = warehouse
        self.source = source

    def seed(
        self, *, fields: list[dict[str, Any]], issues: list[dict[str, Any]], events: list[dict[str, Any]] | None = None
    ) -> None:
        self.warehouse.insert("bronze_jira.jira_fields", self._stamp(fields))
        self.warehouse.insert("bronze_jira.jira_issue", self._stamp(issues))
        self.warehouse.insert("bronze_jira.jira_issue_history", self._stamp(events or []))

    def _stamp(self, records: list[dict[str, Any]]) -> list[dict[str, Any]]:
        """Re-address the builders' rows to this scenario's source.

        `helpers` writes one constant source id, and `unique_key` is built from
        it — two scenarios seeding the same field id or changelog id would
        otherwise collide in a ReplacingMergeTree and silently lose a row.
        """
        if self.source == SOURCE_ID:
            return records
        return [
            {**r, "source_id": self.source, "unique_key": r["unique_key"].replace(SOURCE_ID, self.source, 1)}
            for r in records
        ]

    def build(self, selector: str = FIELD_HISTORY_SELECTOR) -> None:
        self.warehouse.build(selector)

    def journal(self, *, issue: str | None = None, field: str | None = None) -> list[dict[str, Any]]:
        """The journal rows, ordered the way a reader reconstructs history.

        FINAL because the model is a ReplacingMergeTree and unmerged parts would
        otherwise read as duplicates.
        """
        # One constant statement, every filter switched off by its own
        # parameter. Composing the WHERE from pieces would put a formatted
        # string in front of the driver for no gain — the values are bound
        # either way, and a query that is never assembled cannot be assembled
        # wrongly.
        return self.warehouse.rows(
            "SELECT field_id, event_kind, event_id, toString(event_at) AS event_at, _seq,"
            "       field_cardinality, delta_action, value_ids, value_displays, value_id_type,"
            "       author_id"
            " FROM staging.jira__field_history_derived FINAL"
            " WHERE insight_source_id = {src:String}"
            "   AND ({issue:String} = '' OR id_readable = {issue:String})"
            "   AND ({field:String} = '' OR field_id = {field:String})"
            # The reading order, matching the round-trip invariant: the kind
            # first (an initial row is the state at creation, so it precedes any
            # event of the same instant), then `_seq` among the initial rows,
            # then the event id numerically because '101' sorts before '99'.
            " ORDER BY field_id, event_at,"
            "          multiIf(event_kind = 'synthetic_initial', 0,"
            "                  event_kind = 'changelog', 1, 2),"
            "          _seq, toUInt64OrZero(event_id), event_id",
            {"src": self.source, "issue": issue or "", "field": field or ""},
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
        tests find IS a pipeline fault. Reading only success would make every
        scenario that depends on them silently vacuous.
        """
        invocation = self.warehouse.dbt_status("test", "--select", select)
        return invocation.success and "warn" not in invocation.statuses

    def round_trip_holds(self) -> bool:
        """The oracle on its own: does replaying each field land on the value
        the issue holds?"""
        return self.invariants_hold("assert_jira_field_history_round_trip")

    def text_rows(self, text_id: str | None = None) -> list[dict[str, Any]]:
        """The long-text side table, addressed by content hash (§8).

        The table is content-addressed and carries no source, so scenarios
        sharing a build share it: ask for the address the journal gave you.
        """
        return self.warehouse.rows(
            "SELECT text_id, content_form, content"
            " FROM staging.jira__task_field_text FINAL"
            " WHERE ({text:String} = '' OR text_id = {text:String})"
            " ORDER BY content_form, content",
            {"text": text_id or ""},
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


def _truncate_bronze(warehouse: Warehouse) -> None:
    # Spelled out rather than looped over a tuple of names: a table name cannot
    # be bound as a parameter, so a loop would have to format the statement, and
    # a formatted SQL string is the shape the security gate rejects on sight.
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_fields")
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_issue")
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_issue_history")
    warehouse.generation += 1


def _cases(module: Any) -> dict[str, Case]:
    """The module's declared scenarios, keyed by the test that reads each."""
    return {
        name: member.case
        for name, member in vars(module).items()
        if name.startswith("test_") and hasattr(member, "case")
    }


class Batch:
    """A module's declared scenarios, seeded together and built once."""

    def __init__(self, warehouse: Warehouse, module: Any) -> None:
        self.warehouse = warehouse
        self.scenarios = {
            name: Scenario(warehouse, source=f"{SOURCE_ID}-{name[len('test_') :][:60]}") for name in _cases(module)
        }
        self._cases = _cases(module)
        self.generation = -1

    def scenario(self, test: str) -> Scenario:
        """The scenario for one test, building the batch first if it must.

        A test with no `case` truncates bronze for itself, which takes the
        batch's rows with it — so the generation is checked rather than assumed,
        and the batch is rebuilt if an exclusive test ran since.
        """
        if self.generation != self.warehouse.generation:
            self._build()
        return self.scenarios[test]

    def _build(self) -> None:
        _truncate_bronze(self.warehouse)
        for name, scenario in self.scenarios.items():
            spec = self._cases[name]
            scenario.seed(fields=spec.fields, issues=spec.issues, events=spec.events)
        self.warehouse.build()
        self.generation = self.warehouse.generation


@pytest.fixture(scope="module")
def _batch(warehouse: Warehouse, request: pytest.FixtureRequest) -> Batch:
    return Batch(warehouse, request.module)


@pytest.fixture
def scenario(warehouse: Warehouse, request: pytest.FixtureRequest, _batch: Batch) -> Scenario:
    """This test's scenario: one of its module's batch, or a warehouse of its own.

    A test that declares a `case` reads out of the single build its module
    shares. A test that does not — one that builds twice, expects a build to
    fail, or rewrites bronze half way through — gets bronze to itself and seeds
    and builds as it likes, exactly as every test used to.
    """
    if hasattr(request.function, "case"):
        return _batch.scenario(request.function.__name__)
    _truncate_bronze(warehouse)
    return Scenario(warehouse)
