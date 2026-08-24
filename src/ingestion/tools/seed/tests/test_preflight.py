"""Env-contract and preflight-message tests.

Stdlib `unittest` against the real package: env parsing, what a guard counts,
and the messages a refusal carries — the half that has to stay true for an
operator who reads only the error. No server is reached: the one predicate whose
meaning lives in SQL runs against an in-memory sqlite table.

Run against the installed package (see the README's develop section):

    uv run --extra dev pytest tests
"""

from __future__ import annotations

import datetime as _dt
import pathlib
import re
import sqlite3
import unittest
import uuid as uuid_mod
from dataclasses import dataclass

from insight_seed import config, identity, preflight, profiles
from insight_seed.generators import base, insert

_TENANT = "3f1d8f4e-6c2a-4a9b-91d7-8e5c0b2a7f36"

_SEEDED_PERSON = "aaaaaaaa-0000-0000-0000-000000000010"
_UNSEEDED_PERSON = "bbbbbbbb-0000-0000-0000-000000000001"


class _CapturingCursor:
    """Minimal cursor stand-in: records statements, replays one row."""

    def __init__(self, result: tuple[object, ...] | None) -> None:
        self.executed: list[tuple[str, tuple[object, ...]]] = []
        self._result = result

    def execute(self, sql: str, params: tuple[object, ...] = ()) -> None:
        self.executed.append((sql, params))

    def fetchone(self) -> tuple[object, ...] | None:
        return self._result


class TenantContractTests(unittest.TestCase):
    def test_a_missing_tenant_is_refused_and_names_the_variable(self) -> None:
        with self.assertRaises(config.EnvContractError) as caught:
            config.parse_tenant_id({})
        self.assertIn(config.TENANT_ENV, str(caught.exception))

    def test_a_blank_tenant_is_the_same_as_a_missing_one(self) -> None:
        for value in ("", "   ", "\t"):
            with self.subTest(value=value), self.assertRaises(config.EnvContractError):
                config.parse_tenant_id({config.TENANT_ENV: value})

    def test_a_tenant_that_is_not_a_uuid_is_refused(self) -> None:
        with self.assertRaises(config.EnvContractError) as caught:
            config.parse_tenant_id({config.TENANT_ENV: "the-default-one"})
        self.assertIn("not a UUID", str(caught.exception))

    def test_a_uuid_tenant_is_returned_stripped(self) -> None:
        self.assertEqual(config.parse_tenant_id({config.TENANT_ENV: f" {_TENANT} "}), _TENANT)


class AnalyticsDatabaseContractTests(unittest.TestCase):
    def test_a_missing_analytics_database_is_refused_and_explains_why(self) -> None:
        with self.assertRaises(config.EnvContractError) as caught:
            config.parse_analytics_database({})
        message = str(caught.exception)
        self.assertIn(config.ANALYTICS_DB_ENV, message)
        self.assertIn("metric_definitions", message)

    def test_the_identity_database_keeps_a_default_because_every_stand_agrees(self) -> None:
        self.assertEqual(config.parse_identity_database({}), "identity")
        self.assertEqual(config.parse_identity_database({config.IDENTITY_DB_ENV: "ident"}), "ident")


class FlagContractTests(unittest.TestCase):
    def test_the_cross_tenant_fixture_is_on_unless_turned_off(self) -> None:
        self.assertTrue(config.cross_tenant_fixture_enabled({}))
        for value in ("0", "false", "NO", "off"):
            with self.subTest(value=value):
                self.assertFalse(
                    config.cross_tenant_fixture_enabled({config.CROSS_TENANT_FIXTURE_ENV: value})
                )

    def test_force_is_off_unless_asked_for(self) -> None:
        self.assertFalse(config.force_enabled({}))
        self.assertTrue(config.force_enabled({config.FORCE_ENV: "1"}))

    def test_a_flag_that_is_neither_true_nor_false_is_refused(self) -> None:
        with self.assertRaises(config.EnvContractError):
            config.force_enabled({config.FORCE_ENV: "maybe"})


class RefusalMessageTests(unittest.TestCase):
    def test_a_wrong_database_refusal_names_the_database_it_looked_in(self) -> None:
        message = preflight.table_missing_problem(
            "wrong_db", "metric_definitions", needed_for="the analytics catalogue"
        )
        self.assertIn("wrong_db", message)
        self.assertIn("metric_definitions", message)

    def test_a_foreign_rows_refusal_offers_the_override_by_name(self) -> None:
        message = preflight.foreign_rows_problem(7, _TENANT, "identity")
        self.assertIn("7", message)
        self.assertIn(_TENANT, message)
        self.assertIn(config.FORCE_ENV, message)

    def test_a_refusal_reports_every_problem_not_just_the_first(self) -> None:
        error = preflight.PreflightError(["first problem", "second problem"])
        self.assertIn("first problem", str(error))
        self.assertIn("second problem", str(error))


class _FakeResult:
    def __init__(self, rows: list[tuple[object, ...]]) -> None:
        self.result_rows = rows


class _FakeClickHouse:
    """Replays the column lookup, then the per-target counts."""

    def __init__(
        self,
        columns: list[tuple[str, str, str]],
        counts: list[tuple[str, int]],
        total_rows: int = 0,
        tables: list[tuple[str, str]] | None = None,
    ) -> None:
        self._columns = columns
        self._counts = counts
        self._total_rows = total_rows
        # None means "the stand holds every target", which is the ordinary case.
        self._tables = tables
        self.queries: list[str] = []

    def query(self, sql: str, parameters: dict[str, object] | None = None) -> _FakeResult:
        self.queries.append(sql)
        if "system.columns" in sql:
            return _FakeResult(list(self._columns))
        if "system.tables" in sql:
            from insight_seed.generators.insert import RESET_TARGETS

            present = list(RESET_TARGETS) if self._tables is None else self._tables
            return _FakeResult([(schema, table) for schema, table in present])
        if sql.startswith("SELECT sum(n)"):
            return _FakeResult([(self._total_rows,)])
        return _FakeResult([(name, count) for name, count in self._counts])


class SilverResetGuardTests(unittest.TestCase):
    def test_the_scan_covers_exactly_what_the_generators_clear(self) -> None:
        """A name pattern would both miss targets in other databases and refuse
        stands over tables the seed never touches."""
        from insight_seed.generators.insert import RESET_TARGETS

        columns = [(schema, table, "tenant_id") for schema, table in RESET_TARGETS]
        client = _FakeClickHouse(columns=columns, counts=[])
        preflight._foreign_silver_rows(client, _TENANT)

        scan = client.queries[-1]
        for schema, table in RESET_TARGETS:
            with self.subTest(target=f"{schema}.{table}"):
                self.assertIn(f"`{schema}`.`{table}`", scan)
        self.assertNotIn("class_collab_document_activity", scan)

    def test_every_registered_target_is_actually_truncated_by_a_generator(self) -> None:
        """The registry is what preflight scans, so a target that no generator
        clears would refuse a stand over data the seed leaves alone."""
        from insight_seed.generators.insert import RESET_TARGETS

        called: set[tuple[str, str]] = set()
        for path in sorted(pathlib.Path(base.__file__).parent.glob("*.py")):
            for schema, table in re.findall(
                r'truncate\(\s*client,\s*"([a-z0-9_]+)",\s*"([a-z0-9_]+)"', path.read_text()
            ):
                called.add((schema, table))
        self.assertEqual(called, set(RESET_TARGETS))

    def test_a_tenant_column_is_found_under_either_name(self) -> None:
        client = _FakeClickHouse(
            columns=[
                ("silver", "class_focus_metrics", "insight_tenant_id"),
                ("silver", "class_people", "tenant_id"),
                ("silver", "not_a_target", "tenant_id"),
            ],
            counts=[],
        )
        found = preflight._tenant_columns(client)
        self.assertEqual(found[("silver", "class_focus_metrics")], "insight_tenant_id")
        self.assertEqual(found[("silver", "class_people")], "tenant_id")
        self.assertNotIn(("silver", "not_a_target"), found)

    def test_targets_without_a_tenant_column_are_reported_as_unjudgeable(self) -> None:
        from insight_seed.generators.insert import RESET_TARGETS

        client = _FakeClickHouse(
            columns=[("silver", "class_people", "tenant_id")],
            counts=[],
        )
        _, _, unattributable = preflight._foreign_silver_rows(client, _TENANT)
        self.assertIn(("bronze_bamboohr", "employees"), unattributable)
        self.assertNotIn(("silver", "class_people"), unattributable)
        self.assertEqual(len(unattributable), len(RESET_TARGETS) - 1)

    def test_foreign_rows_are_summed_and_the_worst_targets_named(self) -> None:
        client = _FakeClickHouse(
            columns=[
                ("silver", "class_git_commits", "tenant_id"),
                ("silver", "class_people", "tenant_id"),
            ],
            counts=[("silver.class_git_commits", 900), ("silver.class_people", 100)],
        )
        total, worst, _ = preflight._foreign_silver_rows(client, _TENANT)
        self.assertEqual(total, 1000)
        self.assertEqual(worst[0], ("silver.class_git_commits", 900))

    def test_a_null_tenant_is_this_seeder_s_own_row_not_a_foreign_one(self) -> None:
        """`class_people` carries the tenant in `workspace_id`; counting NULLs
        would refuse every re-seed of a stand this seeder seeded itself."""
        client = _FakeClickHouse(columns=[("silver", "class_people", "tenant_id")], counts=[])
        preflight._foreign_silver_rows(client, _TENANT)
        self.assertIn("IS NOT NULL", client.queries[-1])
        self.assertNotIn("IS NULL OR", client.queries[-1])

    def test_a_stand_with_none_of_those_tables_reports_nothing(self) -> None:
        client = _FakeClickHouse(columns=[], counts=[])
        total, worst, _ = preflight._foreign_silver_rows(client, _TENANT)
        self.assertEqual((total, worst), (0, []))

    def test_the_refusal_says_the_step_truncates_and_offers_the_override(self) -> None:
        message = preflight.foreign_silver_problem(1000, [("silver.class_people", 1000)], _TENANT)
        self.assertIn("TRUNCATE", message)
        self.assertIn("class_people", message)
        self.assertIn(config.FORCE_ENV, message)


class ResetRegistryTests(unittest.TestCase):
    def test_clearing_an_unregistered_relation_is_refused(self) -> None:
        with self.assertRaises(ValueError) as caught:
            # The client is never reached: registration is checked first.
            insert.truncate(object(), "silver", "class_not_registered")  # type: ignore[arg-type]
        self.assertIn("RESET_TARGETS", str(caught.exception))


class WindowContractTests(unittest.TestCase):
    """The window has one reader, so the manifest cannot report a window the rows
    are not in — two copies computing `now()` disagree across a UTC midnight."""

    def test_the_generators_and_the_manifest_read_the_same_window(self) -> None:
        from insight_seed import manifest as manifest_mod

        env = {config.ANCHOR_ENV: "2026-06-30", config.DAYS_ENV: "14"}
        self.assertEqual(config.parse_anchor_date(env), _dt.date(2026, 6, 30))
        self.assertEqual(config.parse_seed_days(env), 14)
        self.assertIs(manifest_mod._anchor, config.parse_anchor_date)
        self.assertIs(manifest_mod._days, config.parse_seed_days)
        self.assertIs(base.DEFAULT_SEED_DAYS, config.DEFAULT_SEED_DAYS)

    def test_the_literal_today_and_an_unset_anchor_mean_the_same_day(self) -> None:
        self.assertEqual(
            config.parse_anchor_date({config.ANCHOR_ENV: "today"}),
            config.parse_anchor_date({}),
        )

    def test_an_empty_window_means_unset_because_a_rendered_job_passes_empty(self) -> None:
        self.assertEqual(config.parse_seed_days({config.DAYS_ENV: ""}), config.DEFAULT_SEED_DAYS)
        self.assertEqual(
            config.parse_anchor_date({config.ANCHOR_ENV: ""}), config.parse_anchor_date({})
        )

    def test_a_malformed_window_is_refused_before_anything_is_written(self) -> None:
        for env in (
            {config.ANCHOR_ENV: "30-06-2026"},
            {config.DAYS_ENV: "x"},
            {config.DAYS_ENV: "0"},
        ):
            with self.subTest(env=env), self.assertRaises(config.EnvContractError):
                config.parse_anchor_date(
                    env
                ) if config.ANCHOR_ENV in env else config.parse_seed_days(env)

    def test_a_missing_dev_user_email_is_refused_and_says_what_it_anchors(self) -> None:
        with self.assertRaises(config.EnvContractError) as caught:
            config.parse_dev_user_email({})
        self.assertIn(config.DEV_USER_EMAIL_ENV, str(caught.exception))


class ArtifactLocationTests(unittest.TestCase):
    """Generated files go where the CALLER is, never where pip put the code —
    an installed package's own directory is site-packages."""

    def test_the_manifest_defaults_to_the_working_directory(self) -> None:
        self.assertEqual(config.parse_manifest_path({}), pathlib.Path.cwd() / "manifest.json")

    def test_an_explicit_manifest_path_wins(self) -> None:
        self.assertEqual(
            config.parse_manifest_path({config.MANIFEST_PATH_ENV: "/tmp/somewhere.json"}),
            pathlib.Path("/tmp/somewhere.json"),
        )

    def test_neither_artifact_resolves_inside_the_installed_package(self) -> None:
        from insight_seed import manifest as manifest_mod
        from insight_seed import profile_md

        package_dir = pathlib.Path(manifest_mod.__file__).resolve().parent
        for path in (manifest_mod.manifest_path(), profile_md.profile_path()):
            with self.subTest(path=path):
                self.assertFalse(path.resolve().is_relative_to(package_dir))


class GuardCoverageTests(unittest.TestCase):
    """Which guard runs for which step. The silver scan is differential, so on a
    single-tenant stand it returns zero however much real data the tables hold —
    the `persons` signal is what covers that case, and the destructive step has
    to consult it."""

    def test_the_silver_step_also_consults_the_persons_signal(self) -> None:
        source = pathlib.Path(preflight.__file__).read_text()
        gate = '"identity" in requested or "silver" in requested'
        self.assertIn(gate, source)

    def test_the_silver_scan_is_differential_and_says_so(self) -> None:
        """Locks the reason the gate above exists: a same-tenant row is not
        counted, so this scan alone cannot see a single-tenant stand's data."""
        client = _FakeClickHouse(
            columns=[("silver", "class_people", "tenant_id")],
            counts=[],
        )
        preflight._foreign_silver_rows(client, _TENANT)
        self.assertIn("!= {tenant:String}", client.queries[-1])

    def test_the_reset_surface_is_sized_for_the_operator(self) -> None:
        client = _FakeClickHouse(columns=[], counts=[], total_rows=4321)
        self.assertEqual(preflight._reset_surface_rows(client), 4321)

    def test_a_target_the_stand_does_not_hold_yet_is_left_out_of_the_sizing(self) -> None:
        """One absent relation fails the whole UNION, and the staging target is a
        dbt model — it does not exist until the step this runs before has finished."""
        client = _FakeClickHouse(
            columns=[],
            counts=[],
            total_rows=7,
            tables=[("silver", "class_people")],
        )
        self.assertEqual(preflight._reset_surface_rows(client), 7)

        sizing = client.queries[-1]
        self.assertIn("`silver`.`class_people`", sizing)
        self.assertNotIn("claude_team__ai_invoice", sizing)

    def test_a_stand_holding_none_of_the_targets_is_sized_as_nothing(self) -> None:
        """`SELECT sum(n) FROM ()` is a syntax error, so an empty set must not
        become a query at all."""
        client = _FakeClickHouse(columns=[], counts=[], total_rows=7, tables=[])
        self.assertEqual(preflight._reset_surface_rows(client), 0)
        self.assertFalse([q for q in client.queries if q.startswith("SELECT sum(n)")])


class SeedReasonNamespaceTests(unittest.TestCase):
    def test_every_reason_the_identity_seed_writes_carries_the_shared_prefix(self) -> None:
        reasons = [
            value
            for name, value in vars(identity).items()
            if name.startswith("_REASON_") and isinstance(value, str)
        ]
        self.assertTrue(reasons, "identity.py should define its reasons as _REASON_* constants")
        for reason in reasons:
            with self.subTest(reason=reason):
                self.assertTrue(reason.startswith(config.SEED_REASON_PREFIX))

    def test_a_tenant_with_no_foreign_rows_reads_as_zero(self) -> None:
        self.assertEqual(
            preflight._count_foreign_persons(_CapturingCursor(result=None), _TENANT),  # type: ignore[arg-type]
            0,
        )


@dataclass(frozen=True)
class _PersonsRow:
    """One journal row. The defaults are the stand suite's own: its resolution
    round trip binds a scratch account under its fixed connector instance, in the
    operator persona's name and about a person outside the demo roster."""

    tenant: str = _TENANT
    source_type: str = config.STAND_SCRATCH_SOURCE_TYPE
    source_id: str = config.STAND_SCRATCH_SOURCE_ID
    value_id: str | None = f"{config.STAND_SCRATCH_PREFIX}-a1b2c3d4-resolution-e5f6a7b8"
    value_full_text: str | None = None
    value: str | None = None
    reason: str = "operator-bind"
    person_id: str = _UNSEEDED_PERSON
    author_person_id: str = profiles.ADMIN_OPERATOR_UUID


class _SqlitePersons:
    """`persons` under an engine, over the columns the guard's predicate reads.

    Not a schema replica: the table belongs to the identity service's migrations
    (`001_persons.sql`), which this package cannot see at test time. What is
    modelled is what the predicate reads — `value_effective` as the generated
    column it really is, so a row cannot disagree with its own `value_id`, and
    the uuid columns as the bytes `BINARY(16)` compares.

    Set semantics only: sqlite compares TEXT case-sensitively where MariaDB's
    `utf8mb4_unicode_ci` does not, so no case here turns on letter case.

    Every instance starts as a stand this seeder HAS seeded — one roster row about
    `_SEEDED_PERSON`, without which the projection's exemption cannot be read.
    """

    def __init__(self) -> None:
        self._db = sqlite3.connect(":memory:")
        self._db.execute(
            "CREATE TABLE persons ("
            " insight_tenant_id BLOB NOT NULL,"
            " insight_source_type TEXT NOT NULL,"
            " insight_source_id BLOB NOT NULL,"
            " value_id TEXT,"
            " value_full_text TEXT,"
            " value TEXT,"
            " value_effective TEXT GENERATED ALWAYS AS"
            "  (COALESCE(value_id, value_full_text, value)) STORED,"
            " person_id BLOB NOT NULL,"
            " author_person_id BLOB NOT NULL,"
            " reason TEXT NOT NULL DEFAULT ''"
            ")"
        )
        self._answer: tuple[object, ...] | None = None
        self.insert(
            _PersonsRow(
                source_type=profiles.DEV_SEED_SOURCE_TYPE,
                source_id=profiles.DEV_SEED_SOURCE_ID,
                value_id="seeded@company.nonpresent",
                reason=f"{config.SEED_REASON_PREFIX}demo roster",
                person_id=_SEEDED_PERSON,
                author_person_id=config.SYSTEM_AUTHOR_ID,
            )
        )

    def insert(self, row: _PersonsRow) -> None:
        self._db.execute(
            "INSERT INTO persons (insight_tenant_id, insight_source_type, insight_source_id,"
            " value_id, value_full_text, value, person_id, author_person_id, reason)"
            " VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (
                uuid_mod.UUID(row.tenant).bytes,
                row.source_type,
                uuid_mod.UUID(row.source_id).bytes,
                row.value_id,
                row.value_full_text,
                row.value,
                uuid_mod.UUID(row.person_id).bytes,
                uuid_mod.UUID(row.author_person_id).bytes,
                row.reason,
            ),
        )

    def execute(self, sql: str, params: tuple[object, ...] = ()) -> None:
        self._answer = self._db.execute(sql.replace("%s", "?"), params).fetchone()

    def fetchone(self) -> tuple[object, ...] | None:
        return self._answer


#: `(what the row is, the row, whether the guard must count it)`.
_FOREIGN_ROW_CASES: tuple[tuple[str, _PersonsRow, int], ...] = (
    ("the suite's bind row", _PersonsRow(), 0),
    ("the suite's exclude row", _PersonsRow(reason="operator-exclude"), 0),
    (
        "the suite's prefix carried in value_full_text rather than value_id",
        _PersonsRow(value_id=None, value_full_text=f"{config.STAND_SCRATCH_PREFIX}-a1b2-name"),
        0,
    ),
    ("the same prefix from another connector type", _PersonsRow(source_type="gitlab"), 1),
    (
        "the same prefix from another instance of the same connector",
        _PersonsRow(source_id="01900000-0000-7000-8000-0000000000ff"),
        1,
    ),
    (
        "an operator correction on the suite's own connector instance",
        _PersonsRow(value_id="someone@example.com"),
        1,
    ),
    (
        "a foreign row carrying no value at all",
        _PersonsRow(
            source_type="bamboohr",
            source_id="01900000-0000-7000-8000-0000000000aa",
            value_id=None,
            reason="directory sync",
        ),
        1,
    ),
    (
        "the persons-seed's email link",
        _PersonsRow(value_id="someone@example.com", reason=config.PERSONS_SEED_LINK_REASON),
        0,
    ),
    (
        "this seeder's own roster row",
        _PersonsRow(
            value_id="someone@example.com",
            reason=f"{config.SEED_REASON_PREFIX}roster",
            person_id=_SEEDED_PERSON,
            author_person_id=config.SYSTEM_AUTHOR_ID,
        ),
        0,
    ),
    (
        "another tenant's operator correction",
        _PersonsRow(tenant="ffffffff-ffff-4fff-8fff-ffffffffffff", value_id="a@example.com"),
        0,
    ),
    (
        "the projection re-emitting an attribute of a person this seeder created",
        _PersonsRow(
            source_type="bamboohr",
            source_id="01900000-0000-7000-8000-0000000000aa",
            value_id=None,
            value="Support",
            reason="",
            person_id=_SEEDED_PERSON,
            author_person_id=config.SYSTEM_AUTHOR_ID,
        ),
        0,
    ),
    (
        "the same re-emission about somebody this seeder never created",
        _PersonsRow(
            source_type="bamboohr",
            source_id="01900000-0000-7000-8000-0000000000aa",
            value_id=None,
            value="Support",
            reason="",
            author_person_id=config.SYSTEM_AUTHOR_ID,
        ),
        1,
    ),
    (
        "the projection minting a person from a roster account with no address",
        _PersonsRow(
            source_type="bamboohr",
            source_id="01900000-0000-7000-8000-0000000000aa",
            value_id="42",
            reason="roster-mint",
            person_id=_SEEDED_PERSON,
            author_person_id=config.SYSTEM_AUTHOR_ID,
        ),
        0,
    ),
    (
        "an operator's own correction about a person this seeder created",
        _PersonsRow(
            source_type="bamboohr",
            source_id="01900000-0000-7000-8000-0000000000aa",
            value_id="someone@example.com",
            person_id=_SEEDED_PERSON,
        ),
        1,
    ),
)


class ForeignPersonsPredicateTests(unittest.TestCase):
    """What the guard counts, run against an engine rather than asserted about
    the SQL text. The refusal it drives destroys nothing by itself, but it is
    the only thing standing between the silver step's TRUNCATE and a stand that
    holds somebody's directory — so both directions are the contract: a writer
    this seeder accounts for must not refuse, and everything else must."""

    def test_each_row_is_counted_only_when_no_known_writer_accounts_for_it(self) -> None:
        for description, row, expected in _FOREIGN_ROW_CASES:
            with self.subTest(row=description):
                persons = _SqlitePersons()
                persons.insert(row)
                self.assertEqual(
                    preflight._count_foreign_persons(persons, _TENANT),  # type: ignore[arg-type]
                    expected,
                    f"should count as foreign: {bool(expected)} — {description}",
                )

    def test_the_whole_table_at_once_counts_the_same_rows(self) -> None:
        """Every case above meets the predicate alone; together they also must,
        or one exemption is subtracting rows another writer owns."""
        persons = _SqlitePersons()
        for _, row, _ in _FOREIGN_ROW_CASES:
            persons.insert(row)

        self.assertEqual(
            preflight._count_foreign_persons(persons, _TENANT),  # type: ignore[arg-type]
            sum(expected for _, _, expected in _FOREIGN_ROW_CASES),
        )


def test_a_person_seeded_in_another_tenant_does_not_exempt_this_one() -> None:
    """The exemption's person set is read within the tenant under test. Read
    across tenants, one demo roster would exempt the projection's rows in every
    other tenant on the cluster — including one holding real people."""
    persons = _SqlitePersons()
    stranger = "bbbbbbbb-0000-0000-0000-000000000002"
    elsewhere = "ffffffff-ffff-4fff-8fff-ffffffffffff"
    persons.insert(
        _PersonsRow(
            tenant=elsewhere,
            source_type=profiles.DEV_SEED_SOURCE_TYPE,
            source_id=profiles.DEV_SEED_SOURCE_ID,
            value_id="elsewhere@company.nonpresent",
            reason=f"{config.SEED_REASON_PREFIX}demo roster",
            person_id=stranger,
            author_person_id=config.SYSTEM_AUTHOR_ID,
        )
    )
    persons.insert(
        _PersonsRow(
            source_type="bamboohr",
            source_id="01900000-0000-7000-8000-0000000000aa",
            value_id=None,
            value="Support",
            reason="",
            person_id=stranger,
            author_person_id=config.SYSTEM_AUTHOR_ID,
        )
    )

    assert preflight._count_foreign_persons(persons, _TENANT) == 1  # type: ignore[arg-type]


def _constant_in(path: pathlib.Path, name: str) -> str | None:
    """The string a `NAME = "value"` assignment binds, annotation or not. The
    name is anchored on both sides: without the `\\s*(?::...)?=`, `SCRATCH_PREFIX`
    also matches a `SCRATCH_PREFIX_ANYTHING` declared above the real one."""
    match = re.search(rf'^{name}\s*(?::[^=]*)?=\s*"([^"]+)"', path.read_text(), re.MULTILINE)
    return match.group(1) if match else None


def _find_upwards(*parts: str) -> pathlib.Path | None:
    """The named file in the nearest ancestor that holds it, searching no
    further than the repository root — outside a checkout (the seed image runs
    these tests from the installed package) there is nothing to find, and a
    same-named file above the root would be somebody else's."""
    for parent in pathlib.Path(__file__).resolve().parents:
        candidate = parent.joinpath(*parts)
        if candidate.is_file():
            return candidate
        if (parent / ".git").exists():
            break
    return None


class WriterNamespaceParityTests(unittest.TestCase):
    """Each accounted-for writer spells its namespace out on its own side, and a
    drift brings the refusal back with no test failing anywhere near the change.
    Read the other trees rather than import them: only one language of the three
    is importable here, and the seed image ships none of them — which is a skip,
    not an error."""

    def test_every_scratch_constant_matches_the_stand_suite_s_own(self) -> None:
        scratch = _find_upwards("tests", "stand", "api", "scratch.py")
        if scratch is None:
            self.skipTest("tests/stand/api/scratch.py is not in this tree")

        for mine, theirs in (
            (config.STAND_SCRATCH_PREFIX, "SCRATCH_PREFIX"),
            (config.STAND_SCRATCH_SOURCE_TYPE, "SCRATCH_SOURCE_TYPE"),
            (config.STAND_SCRATCH_SOURCE_ID, "SCRATCH_SOURCE_ID"),
        ):
            with self.subTest(constant=theirs):
                self.assertEqual(_constant_in(scratch, theirs), mine, f"{theirs} drifted")

    def test_the_projection_still_stamps_the_author_this_guard_reads(self) -> None:
        runner = _find_upwards(
            "src", "backend", "services", "identity-resolution", "src", "seed_runner.rs"
        )
        if runner is None:
            self.skipTest("the identity-resolution tree is not in this tree")

        source = runner.read_text()
        self.assertRegex(source, r"SYSTEM_AUTHOR:\s*Uuid\s*=\s*Uuid::nil\(\)")
        self.assertEqual(uuid_mod.UUID(config.SYSTEM_AUTHOR_ID), uuid_mod.UUID(int=0))

        seeded = re.search(r"seed_from_rows\((?P<args>[^;]*?)\)\s*\.await", source, re.S)
        self.assertIsNotNone(seeded, "the projection no longer runs through seed_from_rows")
        self.assertIn(
            "SYSTEM_AUTHOR",
            seeded.group("args"),  # type: ignore[union-attr]
            "the projection must PASS the author this guard reads, not merely declare it: "
            "a run that stamps a real person leaves every re-seed refused",
        )


if __name__ == "__main__":
    unittest.main()
