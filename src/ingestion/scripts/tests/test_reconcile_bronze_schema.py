"""Unit tests for the bronze snapshot reconciler (issue #1991).

The reconciler's I/O is injected, so the whole algorithm is exercised here
against an in-memory stand-in for ClickHouse — no container, no network. The
stand-in answers exactly the three statements the reconciler issues (existence
probe, missing-column diff, type-drift diff) and applies DROP/CREATE/ALTER to
its own state, so assertions are about the schema that results rather than about
SQL strings.
"""

from __future__ import annotations

import importlib.util
import re
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import reconcile_bronze_schema as rbs  # noqa: E402
from reconcile_bronze_schema import (  # noqa: E402
    PROBE_SUFFIX,
    SnapshotTable,
    is_reconcilable,
    load_snapshot_tables,
    parse_snapshot_tables,
    reconcile,
)

LEGACY_COMMITS = {
    "hash": "String",
    "date": "String",
    "author_email": "Nullable(String)",
    "project_key": "Nullable(String)",
}

SNAPSHOT_COMMITS = """CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.commits
(
    `hash` String,
    `date` String,
    `author_email` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `repository_uuid` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY hash
SETTINGS allow_nullable_key = 1, index_granularity = 8192;"""


class FakeClickHouse:
    """Minimal ClickHouse stand-in: tracks {(db, table): {column: type}}."""

    _COLUMN_RE = re.compile(r"^\s*`(?P<name>[^`]+)`\s+(?P<type>.+?),?\s*$")

    def __init__(self, tables: dict[tuple[str, str], dict[str, str]]) -> None:
        self.tables = {key: dict(value) for key, value in tables.items()}
        self.executed: list[str] = []

    # -- injected callables -------------------------------------------------
    def execute(self, sql: str) -> None:
        self.executed.append(sql)
        if sql.startswith("DROP TABLE IF EXISTS"):
            self.tables.pop(self._target(sql), None)
        elif sql.startswith("CREATE TABLE"):
            self.tables[self._target(sql)] = self._columns_of(sql)
        elif sql.startswith("ALTER TABLE"):
            key = self._target(sql)
            match = re.search(r"ADD COLUMN IF NOT EXISTS `(?P<name>[^`]+)` (?P<type>.+)$", sql)
            assert match, sql
            self.tables[key].setdefault(match.group("name"), match.group("type"))
        else:  # pragma: no cover — guards against a silently ignored statement
            raise AssertionError(f"unexpected statement: {sql}")

    def fetch_rows(self, sql: str) -> list[list[str]]:
        if "FROM system.tables" in sql:
            wanted = set(re.findall(r"'([^']+)'", sql))
            return [[db, tbl] for (db, tbl) in sorted(self.tables) if db in wanted]
        probe, live = self._diff_targets(sql)
        if "NOT IN" in sql:
            return [
                [name, ch_type]
                for name, ch_type in self.tables.get(probe, {}).items()
                if name not in self.tables.get(live, {})
            ]
        return [
            [name, ch_type, self.tables[live][name]]
            for name, ch_type in sorted(self.tables.get(probe, {}).items())
            if name in self.tables.get(live, {}) and self.tables[live][name] != ch_type
        ]

    # -- helpers ------------------------------------------------------------
    @staticmethod
    def _target(sql: str) -> tuple[str, str]:
        match = re.search(r"`(?P<db>[^`]+)`\.`(?P<table>[^`]+)`", sql)
        assert match, sql
        return match.group("db"), match.group("table")

    def _diff_targets(self, sql: str) -> tuple[tuple[str, str], tuple[str, str]]:
        names = re.findall(r"table = '([^']+)'", sql)
        db = re.findall(r"database = '([^']+)'", sql)[0]
        probe = next(n for n in names if n.endswith(PROBE_SUFFIX))
        live = next(n for n in names if not n.endswith(PROBE_SUFFIX))
        return (db, probe), (db, live)

    @classmethod
    def _columns_of(cls, create_sql: str) -> dict[str, str]:
        columns: dict[str, str] = {}
        body = create_sql[create_sql.index("(") + 1 : create_sql.rindex(")")]
        for line in body.splitlines():
            match = cls._COLUMN_RE.match(line)
            if match:
                columns[match.group("name")] = match.group("type").rstrip(",").strip()
        return columns

    # -- assertions ---------------------------------------------------------
    def alters(self) -> list[str]:
        return [s for s in self.executed if s.startswith("ALTER TABLE")]


def run(tables, snapshot=(SNAPSHOT_COMMITS,)):
    fake = FakeClickHouse(tables)
    parsed = [t for sql in snapshot for t in parse_snapshot_tables(sql)]
    result = reconcile(parsed, execute=fake.execute, fetch_rows=fake.fetch_rows)
    return fake, result


class TestSnapshotParsing:
    def test_extracts_create_table_statements(self):
        tables = parse_snapshot_tables(SNAPSHOT_COMMITS)
        assert [(t.database, t.table) for t in tables] == [("bronze_bitbucket_cloud", "commits")]

    def test_skips_databases_views_and_materialized_views(self):
        sql = (
            "CREATE DATABASE IF NOT EXISTS `bronze_x`;\n\n"
            f"{SNAPSHOT_COMMITS}\n\n"
            "CREATE OR REPLACE VIEW insight.v AS SELECT 1;\n\n"
            "CREATE MATERIALIZED VIEW IF NOT EXISTS insight.mv REFRESH EVERY 1 HOUR AS SELECT 1;"
        )
        assert [t.table for t in parse_snapshot_tables(sql)] == ["commits"]

    def test_preserves_mixed_case_table_names(self):
        sql = "CREATE TABLE IF NOT EXISTS bronze_salesforce.OpportunityContactRole\n(\n    `Id` String\n)\nENGINE = MergeTree\nORDER BY Id;"
        assert parse_snapshot_tables(sql)[0].table == "OpportunityContactRole"

    def test_reads_every_snapshot_file(self, tmp_path):
        (tmp_path / "a.sql").write_text(SNAPSHOT_COMMITS)
        (tmp_path / "b.sql").write_text(
            "CREATE TABLE IF NOT EXISTS bronze_other.things\n(\n    `id` String\n)\nENGINE = MergeTree\nORDER BY id;"
        )
        assert {t.table for t in load_snapshot_tables(tmp_path)} == {"commits", "things"}


class TestScope:
    def test_only_bronze_databases(self):
        assert is_reconcilable(SnapshotTable("bronze_x", "t", "")) is True
        for database in ("silver", "insight", "staging", "identity", "person"):
            assert is_reconcilable(SnapshotTable(database, "t", "")) is False

    def test_never_reconciles_a_probe_leftover(self):
        assert is_reconcilable(SnapshotTable("bronze_x", f"t{PROBE_SUFFIX}", "")) is False

    def test_non_bronze_tables_are_untouched(self):
        snapshot = SNAPSHOT_COMMITS.replace("bronze_bitbucket_cloud.commits", "silver.class_git_commits")
        fake, result = run({("silver", "class_git_commits"): {"hash": "String"}}, snapshot=(snapshot,))
        assert fake.alters() == []
        assert result.tables_examined == 0


class TestProbeStatement:
    def test_retargets_name_and_preserves_the_rest(self):
        table = parse_snapshot_tables(SNAPSHOT_COMMITS)[0]
        probe_sql = table.probe_sql()
        assert probe_sql.startswith("CREATE TABLE `bronze_bitbucket_cloud`.`commits__ddl_probe`")
        assert "IF NOT EXISTS" not in probe_sql.splitlines()[0]
        assert "ENGINE = ReplacingMergeTree(_airbyte_extracted_at)" in probe_sql
        assert "SETTINGS allow_nullable_key = 1, index_granularity = 8192" in probe_sql
        assert "`bucket_id` Nullable(Int64)" in probe_sql


class TestReconcile:
    def test_adds_columns_missing_from_a_legacy_table(self):
        """The #1991 case: a pre-envelope table gains bucket_id and friends."""
        fake, result = run({("bronze_bitbucket_cloud", "commits"): LEGACY_COMMITS})
        live = fake.tables[("bronze_bitbucket_cloud", "commits")]
        for column in ("record_type", "generation_id", "bucket_id", "repository_uuid"):
            assert column in live, column
        assert live["bucket_id"] == "Nullable(Int64)"
        assert result.columns_added == 4
        assert result.tables_reconciled == 1

    def test_keeps_live_columns_absent_from_the_snapshot(self):
        fake, _ = run({("bronze_bitbucket_cloud", "commits"): LEGACY_COMMITS})
        # project_key is a legacy column the snapshot no longer declares.
        assert "project_key" in fake.tables[("bronze_bitbucket_cloud", "commits")]

    def test_absent_table_is_left_to_the_snapshots_own_create(self):
        fake, result = run({})
        assert fake.alters() == []
        assert result.tables_examined == 0

    def test_is_idempotent(self):
        first, first_result = run({("bronze_bitbucket_cloud", "commits"): LEGACY_COMMITS})
        healed = first.tables[("bronze_bitbucket_cloud", "commits")]
        second, second_result = run({("bronze_bitbucket_cloud", "commits"): healed})
        assert first_result.columns_added == 4
        assert second_result.columns_added == 0
        assert second.alters() == []

    def test_reports_type_drift_without_modifying_it(self):
        drifted = dict(LEGACY_COMMITS, author_email="String")
        fake, result = run({("bronze_bitbucket_cloud", "commits"): drifted})
        assert result.type_drift == [
            ("bronze_bitbucket_cloud.commits", "author_email", "Nullable(String)", "String")
        ]
        assert fake.tables[("bronze_bitbucket_cloud", "commits")]["author_email"] == "String"
        assert not any("MODIFY" in sql for sql in fake.executed)

    def test_drops_the_probe_when_done(self):
        fake, _ = run({("bronze_bitbucket_cloud", "commits"): LEGACY_COMMITS})
        assert ("bronze_bitbucket_cloud", f"commits{PROBE_SUFFIX}") not in fake.tables
        assert fake.executed[-1].startswith("DROP TABLE IF EXISTS")

    def test_drops_the_probe_even_when_a_statement_fails(self):
        fake = FakeClickHouse({("bronze_bitbucket_cloud", "commits"): LEGACY_COMMITS})
        real_execute = fake.execute

        def flaky(sql: str) -> None:
            if sql.startswith("ALTER TABLE"):
                raise RuntimeError("boom")
            real_execute(sql)

        with pytest.raises(RuntimeError):
            reconcile(
                parse_snapshot_tables(SNAPSHOT_COMMITS),
                execute=flaky,
                fetch_rows=fake.fetch_rows,
            )
        assert fake.executed[-1].startswith("DROP TABLE IF EXISTS")
        assert ("bronze_bitbucket_cloud", f"commits{PROBE_SUFFIX}") not in fake.tables

    def test_alter_statements_quote_identifiers(self):
        fake, _ = run({("bronze_bitbucket_cloud", "commits"): LEGACY_COMMITS})
        for sql in fake.alters():
            assert sql.startswith("ALTER TABLE `bronze_bitbucket_cloud`.`commits` ADD COLUMN IF NOT EXISTS `")


class TestLoadableByPath:
    """The e2e rig loads this file by path, not as an installed module.

    Regression test: `module_from_spec` + `exec_module` without registering the
    module in sys.modules first raises `AttributeError: 'NoneType' object has no
    attribute '__dict__'` on the first @dataclass, because dataclass resolves its
    own module through `sys.modules[cls.__module__]`. Importing normally (as the
    tests above do) hides that, so this exercises the rig's actual code path.
    """

    def _load(self, name: str):
        spec = importlib.util.spec_from_file_location(
            name, Path(__file__).resolve().parent.parent / "reconcile_bronze_schema.py"
        )
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        try:
            spec.loader.exec_module(module)
        except Exception:
            sys.modules.pop(spec.name, None)
            raise
        return module

    def test_executes_and_its_dataclasses_are_usable(self):
        module = self._load("reconcile_bronze_schema_pathloaded")
        try:
            table = module.SnapshotTable("bronze_x", "t", SNAPSHOT_COMMITS)
            assert table.probe == f"t{module.PROBE_SUFFIX}"
            assert module.ReconcileResult().columns_added == 0
        finally:
            sys.modules.pop("reconcile_bronze_schema_pathloaded", None)

    def test_reconciles_when_loaded_by_path(self):
        """End-to-end through the path-loaded module, as the rig calls it."""
        module = self._load("reconcile_bronze_schema_pathloaded2")
        try:
            fake = FakeClickHouse({("bronze_bitbucket_cloud", "commits"): LEGACY_COMMITS})
            result = module.reconcile(
                module.parse_snapshot_tables(SNAPSHOT_COMMITS),
                execute=fake.execute,
                fetch_rows=fake.fetch_rows,
            )
            assert result.columns_added == 4
            assert "bucket_id" in fake.tables[("bronze_bitbucket_cloud", "commits")]
        finally:
            sys.modules.pop("reconcile_bronze_schema_pathloaded2", None)


class TestHttpClient:
    """The transport the deploy Job uses — mirrors lib/ch-exec.sh."""

    ENV = {
        "CLICKHOUSE_URL": "http://ch:8123/",
        "CLICKHOUSE_USER": "insight",
        "CLICKHOUSE_PASSWORD": "secret",
    }

    def _stub_urlopen(self, monkeypatch, body: str, seen: list):
        class Response:
            def __enter__(self_inner):
                return self_inner

            def __exit__(self_inner, *exc):
                return False

            def read(self_inner):
                return body.encode("utf-8")

        def urlopen(request):
            seen.append(request)
            return Response()

        monkeypatch.setattr(rbs.urllib.request, "urlopen", urlopen)

    def test_requires_every_credential(self, monkeypatch):
        for missing in self.ENV:
            for key, value in self.ENV.items():
                monkeypatch.setenv(key, value)
            monkeypatch.delenv(missing)
            with pytest.raises(SystemExit, match=missing):
                rbs._http_client()

    def test_posts_credentials_as_headers_not_query(self, monkeypatch):
        for key, value in self.ENV.items():
            monkeypatch.setenv(key, value)
        seen: list = []
        self._stub_urlopen(monkeypatch, "", seen)

        execute, _ = rbs._http_client()
        execute("ALTER TABLE x ADD COLUMN y String")

        request = seen[0]
        assert request.full_url == "http://ch:8123/"
        assert request.data == b"ALTER TABLE x ADD COLUMN y String"
        assert request.get_header("X-clickhouse-user") == "insight"
        assert request.get_header("X-clickhouse-key") == "secret"

    def test_fetch_rows_parses_tsv_and_drops_blank_lines(self, monkeypatch):
        for key, value in self.ENV.items():
            monkeypatch.setenv(key, value)
        self._stub_urlopen(monkeypatch, "bucket_id\tNullable(Int64)\n\nrepo\tString\n", [])

        _, fetch_rows = rbs._http_client()

        assert fetch_rows("SELECT 1") == [["bucket_id", "Nullable(Int64)"], ["repo", "String"]]


class TestCli:
    def test_reconciles_the_given_snapshot_directory(self, monkeypatch, tmp_path, caplog):
        (tmp_path / "bitbucket-cloud.sql").write_text(SNAPSHOT_COMMITS)
        fake = FakeClickHouse({("bronze_bitbucket_cloud", "commits"): LEGACY_COMMITS})
        monkeypatch.setattr(rbs, "_http_client", lambda: (fake.execute, fake.fetch_rows))

        with caplog.at_level("INFO", logger="reconcile_bronze_schema"):
            assert rbs.main([str(tmp_path)]) == 0

        assert "bucket_id" in fake.tables[("bronze_bitbucket_cloud", "commits")]
        assert "reconciled 4 column(s)" in caplog.text

    def test_warns_about_type_drift(self, monkeypatch, tmp_path, caplog):
        (tmp_path / "bitbucket-cloud.sql").write_text(SNAPSHOT_COMMITS)
        fake = FakeClickHouse(
            {("bronze_bitbucket_cloud", "commits"): dict(LEGACY_COMMITS, author_email="String")}
        )
        monkeypatch.setattr(rbs, "_http_client", lambda: (fake.execute, fake.fetch_rows))

        with caplog.at_level("WARNING", logger="reconcile_bronze_schema"):
            rbs.main([str(tmp_path)])

        assert "differ in type" in caplog.text

    def test_missing_snapshot_directory_is_fatal(self, tmp_path):
        with pytest.raises(SystemExit, match="not found"):
            rbs.main([str(tmp_path / "nope")])
