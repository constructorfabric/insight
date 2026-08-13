"""Env-contract and preflight-message tests.

Stdlib `unittest` against the real package: env parsing, the SQL a guard
issues, and the messages a refusal carries — the half that has to stay true
for an operator who reads only the error. No database is touched.

Run against the installed package (see the README's develop section):

    uv run --extra dev python -m unittest discover -s tests -t .
"""

from __future__ import annotations

import datetime as _dt
import pathlib
import re
import unittest
import uuid as uuid_mod

from insight_seed import config, identity, preflight
from insight_seed.generators import base

_TENANT = "3f1d8f4e-6c2a-4a9b-91d7-8e5c0b2a7f36"


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
    ) -> None:
        self._columns = columns
        self._counts = counts
        self._total_rows = total_rows
        self.queries: list[str] = []

    def query(self, sql: str, parameters: dict[str, object] | None = None) -> _FakeResult:
        self.queries.append(sql)
        if "system.columns" in sql:
            return _FakeResult(list(self._columns))
        if sql.startswith("SELECT sum(n)"):
            return _FakeResult([(self._total_rows,)])
        return _FakeResult([(name, count) for name, count in self._counts])


class SilverResetGuardTests(unittest.TestCase):
    def test_the_scan_covers_exactly_what_the_generators_clear(self) -> None:
        """A name pattern would both miss targets in other databases and refuse
        stands over tables the seed never touches."""
        from insight_seed.generators.base import RESET_TARGETS

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
        from insight_seed.generators.base import RESET_TARGETS

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
        from insight_seed.generators.base import RESET_TARGETS

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
            base.truncate(object(), "silver", "class_not_registered")  # type: ignore[arg-type]
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

    def test_the_foreign_row_query_excludes_exactly_that_prefix(self) -> None:
        cursor = _CapturingCursor(result=(3,))
        self.assertEqual(preflight._count_foreign_persons(cursor, _TENANT), 3)  # type: ignore[arg-type]

        sql, params = cursor.executed[-1]
        self.assertIn("reason NOT LIKE", sql)
        self.assertEqual(params[0], uuid_mod.UUID(_TENANT).bytes)
        self.assertEqual(params[1], f"{config.SEED_REASON_PREFIX}%")

    def test_a_tenant_with_no_foreign_rows_reads_as_zero(self) -> None:
        self.assertEqual(
            preflight._count_foreign_persons(_CapturingCursor(result=None), _TENANT),  # type: ignore[arg-type]
            0,
        )


if __name__ == "__main__":
    unittest.main()
