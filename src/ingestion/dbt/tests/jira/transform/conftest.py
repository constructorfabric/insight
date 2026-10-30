"""Rig for the Jira field-history transformation tests.

One ClickHouse, one dbt project, real models. A test seeds the three bronze
tables the chain reads, builds it, and reads the journal back:

    jira_fields + jira_issue + jira_issue_history
        -> dbt build --select tag:jira,tag:staging
        -> staging.jira__field_history_derived

Nothing is stubbed. Bronze is created by the Airbyte destination from the Jira
connector's own catalogue (`tests/bronze.py`), the one creator a deployment has,
so the tables have production's engines and column types — including the
ReplacingMergeTree shape — because they ARE what a first sync would leave.

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
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import clickhouse_connect
import pytest
import yaml
from dbt.cli.main import dbtRunner
from helpers import SOURCE_ID
from phased import run_phased

# tests/jira/transform -> tests/jira -> tests -> dbt
DBT_DIR = Path(__file__).resolve().parents[3]
INGESTION_DIR = DBT_DIR.parent
# This suite's rootdir is its own directory, so the shared helper beside the other
# transform suites is not importable without saying where it is. `dbt_profiles`
# is the deploy's own profile writer, used here so the rig connects the way a
# deployment does.
sys.path.insert(0, str(DBT_DIR / "tests"))
sys.path.insert(0, str(INGESTION_DIR / "scripts"))

from bronze import create_bronze  # noqa: E402
from dbt_profiles import Connection, build_profile  # noqa: E402

# What the prod staging step selects. Building it is its own assertion — that
# the field-history models coexist with every other Jira staging model — and
# `test_invariants` makes it once.
PROD_SELECTOR = "tag:jira,tag:staging"

# What a scenario actually needs: the journal, the side table, and their
# ancestors, including availability and project-visibility snapshots for link
# key resolution. Comment and worklog models remain in the full selector;
# test_invariants builds that chain once.
FIELD_HISTORY_SELECTOR = "+jira__field_history_derived +jira__task_field_text +jira__task_field_unclassified"

# The journal and the links producer: a module about links asserts both, because
# they resolve a linked issue's key through the same map.
LINKS_SELECTOR = f"{FIELD_HISTORY_SELECTOR} +jira__task_links"


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

    def build(
        self,
        selector: str = FIELD_HISTORY_SELECTOR,
        *,
        full_refresh: bool = True,
        dbt_vars: dict[str, Any] | None = None,
    ) -> None:
        # `run`, not `build`: `build` interleaves the singular tests, so a
        # scenario written to make an invariant fail — and there is one, because
        # the failure is the point — would look like a broken model instead.
        # Invariants are asserted explicitly, per scenario, below.
        #
        # `--full-refresh` because two models are incremental and keep state
        # across runs on purpose (`jira__catalogue_first_seen`, the journal):
        # every scenario seeds its own bronze and reads its own answer. The
        # tests that are ABOUT the persistence build again without the flag.
        # The catalogue refuses a full refresh, so it is emptied by hand.
        if full_refresh:
            self.execute("TRUNCATE TABLE IF EXISTS staging.jira__catalogue_first_seen")
        self.dbt(
            "run",
            "--select",
            *selector.split(),
            *(["--full-refresh"] if full_refresh else []),
            *(["--vars", json.dumps(dbt_vars)] if dbt_vars else []),
        )


#: The class relations that must stand before the first test runs: two a module
#: writes into directly (`test_title_role` seeds the journal and an actor,
#: `test_silver_incremental_boundary` plants another producer's row) and two more
#: that `task_issue_state` joins by name, which no test seeds.
SEEDED_CLASSES = ("class_task_field_history", "class_task_users", "class_task_issuetypes", "class_task_statuses")

#: Their Jira arms, built first: `union_by_tag` unions the contributors that are
#: readable and skips the rest, so a class built with none of them would fall back
#: to selecting from a target that does not exist yet. Named per model rather than
#: by class, because the other trackers' arms have no bronze on this rig.
SEEDED_CLASS_ARMS = (
    "+jira__field_history_derived",
    "+jira__task_users",
    "+jira__task_issuetypes",
    "+jira__task_statuses",
)


@pytest.fixture(scope="session")
def warehouse() -> Warehouse:
    """Session setup: databases, bronze, the classes a module seeds, and a dbt
    profile pointing at them.

    Nothing else in silver is pre-created. A class this connector feeds is built
    by the run that asserts over it; one it does not feed is excluded by
    `list_unbuildable_models` rather than standing empty.
    """
    with tempfile.TemporaryDirectory() as tmp:
        profiles_dir = Path(tmp)
        wh = Warehouse(profiles_dir)
        # `password` is inlined rather than read through env_var: this profile is
        # written to a private temp dir for one session and never committed.
        profile = build_profile(
            "test",
            Connection(host=wh.host, port=wh.port, user=wh.user, password=wh.password),
            correlated_subqueries=True,
        )
        profiles_dir.joinpath("profiles.yml").write_text(yaml.safe_dump(profile))
        # Every database this rig touches. A throwaway warehouse makes its own:
        # on a deployment create-databases.sh is the one creation site, and
        # no on-run-start hook carries a CREATE DATABASE.
        wh.execute("CREATE DATABASE IF NOT EXISTS staging")
        wh.execute("CREATE DATABASE IF NOT EXISTS silver")
        wh.execute("CREATE DATABASE IF NOT EXISTS identity")
        wh.execute("CREATE DATABASE IF NOT EXISTS config")
        wh.execute("CREATE DATABASE IF NOT EXISTS insight")
        create_bronze("jira", port=wh.port, user=wh.user, password=wh.password, database="default")
        # Parse here rather than inside whichever test runs first, so a broken
        # project reads as a setup error and costs one test no seconds.
        _ = wh.dbt_project
        wh.build(" ".join(SEEDED_CLASS_ARMS))
        wh.build(" ".join(SEEDED_CLASSES))
        yield wh


@dataclass(frozen=True)
class Case:
    """The bronze one scenario is about, declared beside the test that reads it."""

    fields: list[dict[str, Any]]
    issues: list[dict[str, Any]]
    events: list[dict[str, Any]]
    statuses: list[dict[str, Any]]
    census: list[dict[str, Any]]


def case(
    *,
    fields: list[dict[str, Any]],
    issues: list[dict[str, Any]],
    events: list[dict[str, Any]] | None = None,
    statuses: list[dict[str, Any]] | None = None,
    census: list[dict[str, Any]] | None = None,
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
        test.case = Case(
            fields=fields, issues=issues, events=events or [], statuses=statuses or [], census=census or []
        )
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
        self,
        *,
        fields: list[dict[str, Any]],
        issues: list[dict[str, Any]],
        events: list[dict[str, Any]] | None = None,
        statuses: list[dict[str, Any]] | None = None,
        census: list[dict[str, Any]] | None = None,
        projects: list[dict[str, Any]] | None = None,
    ) -> None:
        self.warehouse.insert("bronze_jira.jira_fields", self._stamp(fields))
        self.warehouse.insert("bronze_jira.jira_issue", self._stamp(issues))
        self.warehouse.insert("bronze_jira.jira_issue_history", self._stamp(events or []))
        self.warehouse.insert("bronze_jira.jira_statuses", self._stamp(statuses or []))
        self.warehouse.insert("bronze_jira.jira_issue_census", self._stamp(census or []))
        self.warehouse.insert("bronze_jira.jira_project_visibility", self._stamp(projects or []))

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

    def build(
        self,
        selector: str = FIELD_HISTORY_SELECTOR,
        *,
        full_refresh: bool = True,
        dbt_vars: dict[str, Any] | None = None,
    ) -> None:
        self.warehouse.build(selector, full_refresh=full_refresh, dbt_vars=dbt_vars)

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
            "SELECT field_id, event_kind, event_id, toString(event_at) AS event_at, _seq, event_order,"
            "       field_cardinality, delta_action, value_ids, value_displays, value_id_type,"
            "       author_id"
            " FROM staging.jira__field_history_derived FINAL"
            " WHERE insight_source_id = {src:String}"
            "   AND ({issue:String} = '' OR id_readable = {issue:String})"
            "   AND ({field:String} = '' OR field_id = {field:String})"
            # The reading order: the class's one ordering key.
            " ORDER BY field_id, event_order",
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
        """This scenario's rows of the long-text side table (§8)."""
        return self.warehouse.rows(
            "SELECT insight_source_id, data_source, text_id, content_form, content"
            " FROM staging.jira__task_field_text FINAL"
            " WHERE insight_source_id = {src:String}"
            "   AND ({text:String} = '' OR text_id = {text:String})"
            " ORDER BY content_form, content",
            {"src": self.source, "text": text_id or ""},
        )

    def links(self, *, issue_id: str | None = None) -> list[dict[str, Any]]:
        """The link intervals `jira__task_links` produced, in a fixed order."""
        return self.warehouse.rows(
            "SELECT issue_id, id_readable, link_type, target_id, target_readable,"
            "       toString(valid_from) AS valid_from, valid_from_known,"
            "       if(valid_to IS NULL, NULL, toString(valid_to)) AS valid_to,"
            "       added_by, removed_by, origin_event_id, toString(evidence) AS evidence,"
            "       is_cross_repository, unique_key"
            " FROM staging.jira__task_links FINAL"
            " WHERE insight_source_id = {src:String}"
            "   AND ({issue:String} = '' OR issue_id = {issue:String})"
            " ORDER BY issue_id, link_type, target_readable, valid_from",
            {"src": self.source, "issue": issue_id or ""},
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


def clear_stateful_link_inputs(warehouse: Warehouse) -> None:
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_issue_census")
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_project_visibility")
    warehouse.execute("TRUNCATE TABLE IF EXISTS staging.jira__issue_availability_snapshot")
    warehouse.execute("TRUNCATE TABLE IF EXISTS staging.jira__project_visibility_snapshot")
    warehouse.execute("TRUNCATE TABLE IF EXISTS staging.jira__issue_link_snapshot")


def _truncate_bronze(warehouse: Warehouse) -> None:
    # Spelled out rather than looped over a tuple of names: a table name cannot
    # be bound as a parameter, so a loop would have to format the statement, and
    # a formatted SQL string is the shape the security gate rejects on sight.
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_fields")
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_issue")
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_issue_history")
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_statuses")
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_comments")
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_issue_keys")
    clear_stateful_link_inputs(warehouse)
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
        self._selector: str = getattr(module, "SELECTOR", FIELD_HISTORY_SELECTOR)
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
            scenario.seed(
                fields=spec.fields, issues=spec.issues, events=spec.events, statuses=spec.statuses, census=spec.census
            )
        self.warehouse.build(self._selector)
        self.generation = self.warehouse.generation


@pytest.fixture(scope="module")
def _phased(warehouse: Warehouse, request: pytest.FixtureRequest) -> dict[str, BaseException | None]:
    items = [
        item
        for item in request.session.items
        if isinstance(item, pytest.Function) and item.module is request.module and hasattr(item.obj, "phased")
    ]
    _truncate_bronze(warehouse)
    try:
        return run_phased(
            items, lambda index: Scenario(warehouse, source=f"{SOURCE_ID}-phased-{index}"), warehouse.build
        )
    finally:
        warehouse.generation += 1


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
