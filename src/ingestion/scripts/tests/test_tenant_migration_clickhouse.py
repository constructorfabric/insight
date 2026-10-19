"""The tenant migration against a real ClickHouse: every key form, every refusal,
and the way back.

Needs a throwaway server — it drops and recreates the databases it seeds:

    TENANT_MIGRATION_TEST_CH_URL=http://localhost:38211 \\
    TENANT_MIGRATION_TEST_CH_USER=insight TENANT_MIGRATION_TEST_CH_PASSWORD=insight \\
    uvx pytest tests/test_tenant_migration_clickhouse.py
"""

from __future__ import annotations

import os
import sys
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from tenant_migration import apply as migration
from tenant_migration.__main__ import main
from tenant_migration.clickhouse import ClickHouse, ClickHouseError
from tenant_migration.inventory import discover, history_relations
from tenant_migration.keys import Tenants
from tenant_migration.plan import Plan, survey
from tenant_migration.prerequisites import fill_youtrack_comment_ids

NEW = "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"
OLD = "example-main-legacy"
DATABASES = (
    "bronze_example",
    "bronze_bamboohr",
    "bronze_jira",
    "bronze_youtrack",
    "staging",
    "config",
    "ingestion_history",
)
RESET = [step for db in DATABASES for step in (f"DROP DATABASE IF EXISTS {db} SYNC", f"CREATE DATABASE {db}")]
TABLES = (
    "bronze_example.items",
    "bronze_example.colon_items",
    "bronze_bamboohr.whos_out",
    "bronze_youtrack.youtrack_comments",
    "staging.jira__catalogue_first_seen",
    "staging.slack__users_snapshot",
    "config.task_value_map",
    "ingestion_history.sync_events",
)

SCHEMA = f"""
CREATE TABLE bronze_example.items (unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    payload String, _airbyte_extracted_at DateTime64(3))
ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key COMMENT 'synthetic items';
INSERT INTO bronze_example.items VALUES
    ('{OLD}-main-PROJ-7', '{OLD}', 'main', 'the old tenant, whose own key holds the source name', '2026-01-02 00:00:00'),
    ('-main-PROJ-7', NULL, 'main', 'the same entity written before the tenant was stamped', '2026-01-01 00:00:00'),
    ('-main-PROJ-8', '', 'main', 'an empty tenant', '2026-01-02 00:00:00'),
    ('{NEW}-main-PROJ-9', '{NEW}', 'main', 'already canonical', '2026-01-02 00:00:00'),
    ('fixture-9', '{OLD}', 'main', 'a key that carries no tenant', '2026-01-02 00:00:00'),
    ('{OLD}-main', '{OLD}', 'main', 'a key that is exactly tenant and source', '2026-01-02 00:00:00');
CREATE TABLE bronze_example.colon_items (unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    _airbyte_extracted_at DateTime64(3))
ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
INSERT INTO bronze_example.colon_items VALUES ('{OLD}:main:org/repo:1', '{OLD}', 'main', '2026-01-02 00:00:00');
CREATE TABLE bronze_bamboohr.whos_out (unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    entries_json String, _airbyte_extracted_at DateTime64(3))
ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
INSERT INTO bronze_bamboohr.whos_out VALUES ('["{OLD}","bamboo"]', '{OLD}', 'bamboo', '[]', '2026-01-02 00:00:00');
CREATE TABLE bronze_youtrack.youtrack_comments (unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    id Nullable(String), _airbyte_extracted_at DateTime64(3))
ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
INSERT INTO bronze_youtrack.youtrack_comments VALUES ('{OLD}-yt-4-17', '{OLD}', 'yt', NULL, '2026-01-02 00:00:00');
CREATE TABLE staging.jira__catalogue_first_seen (unique_key String, tenant_id String, insight_source_id String,
    catalogue_first_sync DateTime64(3), recorded_at DateTime64(3), _version UInt64)
ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key;
INSERT INTO staging.jira__catalogue_first_seen VALUES ('{OLD}-main', '{OLD}', 'main', '2026-01-01 00:00:00', '2026-01-01 00:05:00', 1);
CREATE TABLE staging.slack__users_snapshot (unique_key Nullable(String), tenant_id Nullable(String),
    source_id Nullable(String), name String, _tracked_at DateTime64(3))
ENGINE = MergeTree ORDER BY tuple();
INSERT INTO staging.slack__users_snapshot VALUES
    ('{OLD}-slack-U1', '{OLD}', 'slack', 'first version', '2026-01-01 00:00:00'),
    ('{OLD}-slack-U1', '{OLD}', 'slack', 'second version', '2026-01-02 00:00:00'),
    (NULL, '{OLD}', 'slack', 'a row without a key', '2026-01-02 00:00:00');
CREATE TABLE config.task_value_map (unique_key String, tenant_id String, insight_source_id String, value String,
    _version DateTime64(3))
ENGINE = ReplacingMergeTree(_version) ORDER BY unique_key;
INSERT INTO config.task_value_map VALUES ('{OLD}:main:status:open', '{OLD}', 'main', 'open', '2026-01-01 00:00:00');
CREATE TABLE ingestion_history.sync_events (ts DateTime64(3), tenant_id LowCardinality(String),
    source_id LowCardinality(String), event LowCardinality(String))
ENGINE = MergeTree ORDER BY (event, ts);
INSERT INTO ingestion_history.sync_events VALUES ('2026-01-01 00:00:00', '{OLD}', 'main', 'sync'), ('2026-01-01 00:00:01', '', 'main', 'sync');
"""


@pytest.fixture(scope="module")
def ch() -> ClickHouse:
    url = os.environ.get("TENANT_MIGRATION_TEST_CH_URL")
    if not url:
        pytest.skip("TENANT_MIGRATION_TEST_CH_URL is not set")
    os.environ.update(
        CLICKHOUSE_URL=url,
        CLICKHOUSE_USER=os.environ.get("TENANT_MIGRATION_TEST_CH_USER", "default"),
        CLICKHOUSE_PASSWORD=os.environ.get("TENANT_MIGRATION_TEST_CH_PASSWORD", ""),
    )
    return ClickHouse.from_env()


def _statements(sql: str) -> list[str]:
    return [s.strip() for s in sql.split(";\n") if s.strip()]


@pytest.fixture
def warehouse(ch: ClickHouse) -> ClickHouse:
    for statement in RESET:
        ch.execute(statement)
    for statement in _statements(SCHEMA):
        ch.execute(statement)
    return ch


def _plan(ch: ClickHouse, old: str = OLD) -> Plan:
    return survey(ch, discover(ch, history_relations()), Tenants.parse(NEW, old))


def _fingerprint(ch: ClickHouse) -> dict[str, tuple[str, ...]]:
    return {t: tuple(ch.rows(f"SELECT count(), sum(cityHash64(*)) FROM {t}")[0]) for t in TABLES}


def _apply(ch: ClickHouse, plan: Plan, tmp_path: Path) -> None:
    migration.apply(ch, plan, tmp_path / "dump", before_rewrite=[lambda: fill_youtrack_comment_ids(ch, plan.tenants)])


def _column(ch: ClickHouse, table: str, column: str) -> list[str]:
    database, name = table.split(".")
    replacing = ch.scalar(
        f"SELECT count() FROM system.tables WHERE database = '{database}' AND name = '{name}' AND engine LIKE '%Replacing%'"
    )
    final = "FINAL" if replacing else ""
    return [row[0] for row in ch.rows(f"SELECT ifNull({column}, 'NULL') FROM {table} {final} ORDER BY 1")]


def test_a_plan_counts_what_moves_and_writes_nothing(warehouse: ClickHouse) -> None:
    before = _fingerprint(warehouse)

    plan = _plan(warehouse)

    moving = {t.table.relation: t.moving for t in plan.tables}
    assert plan.refusals == []
    assert moving["bronze_example.items"] == 5, moving
    assert moving["ingestion_history.sync_events"] == 2, moving
    assert plan.comment_ids_to_fill == 1
    assert _fingerprint(warehouse) == before


@pytest.mark.parametrize(
    ("table", "column", "expected"),
    [
        (
            "bronze_example.items",
            "unique_key",
            sorted([f"{NEW}-main-PROJ-7", f"{NEW}-main-PROJ-8", f"{NEW}-main-PROJ-9", "fixture-9", f"{NEW}-main"]),
        ),
        ("bronze_example.items", "tenant_id", [NEW] * 5),
        ("bronze_example.colon_items", "unique_key", [f"{NEW}:main:org/repo:1"]),
        ("bronze_bamboohr.whos_out", "unique_key", [f'["{NEW}","bamboo"]']),
        ("bronze_youtrack.youtrack_comments", "id", ["4-17"]),
        ("staging.jira__catalogue_first_seen", "unique_key", [f"{NEW}-main"]),
        ("staging.slack__users_snapshot", "unique_key", sorted([f"{NEW}-slack-U1", f"{NEW}-slack-U1", "NULL"])),
        ("config.task_value_map", "unique_key", [f"{NEW}:main:status:open"]),
        ("ingestion_history.sync_events", "tenant_id", [NEW, NEW]),
    ],
)
def test_apply_moves_every_key_form_to_the_new_tenant(
    warehouse: ClickHouse, tmp_path: Path, table: str, column: str, expected: list[str]
) -> None:
    _apply(warehouse, _plan(warehouse), tmp_path)

    assert _column(warehouse, table, column) == expected, f"{table}.{column}"


def test_the_entity_written_twice_keeps_its_newest_row(warehouse: ClickHouse, tmp_path: Path) -> None:
    _apply(warehouse, _plan(warehouse), tmp_path)

    payload = warehouse.rows(f"SELECT payload FROM bronze_example.items FINAL WHERE unique_key = '{NEW}-main-PROJ-7'")
    assert payload == [["the old tenant, whose own key holds the source name"]]


def test_a_second_run_waits_for_finalize_and_then_finds_nothing_to_move(warehouse: ClickHouse, tmp_path: Path) -> None:
    _apply(warehouse, _plan(warehouse), tmp_path)

    waiting = _plan(warehouse)
    migration.finalize(warehouse)
    settled = _plan(warehouse)

    assert waiting.changing == [] and any("finalize or roll back first" in r for r in waiting.refusals)
    assert settled.changing == [] and settled.refusals == [], settled.refusals


def test_finalize_gives_a_rewritten_table_its_own_comment_back(warehouse: ClickHouse, tmp_path: Path) -> None:
    _apply(warehouse, _plan(warehouse), tmp_path)

    migration.finalize(warehouse)

    assert _comment(warehouse, "bronze_example", "items") == "synthetic items"


def test_rollback_puts_back_what_was_there_keeping_only_the_comment_ids_it_filled(
    warehouse: ClickHouse, tmp_path: Path
) -> None:
    """The ids are read from the key before it changes, so they hold under either key."""
    before = _fingerprint(warehouse)
    _apply(warehouse, _plan(warehouse), tmp_path)

    migration.rollback(warehouse)

    after = _fingerprint(warehouse)
    others = [t for t in TABLES if t != "bronze_youtrack.youtrack_comments"]
    assert {t: after[t] for t in others} == {t: before[t] for t in others}
    assert _column(warehouse, "bronze_youtrack.youtrack_comments", "unique_key") == [f"{OLD}-yt-4-17"]
    assert _column(warehouse, "bronze_youtrack.youtrack_comments", "id") == ["4-17"]
    assert migration.finalize(warehouse), "the migrated tables are kept as rejected until finalize"
    assert warehouse.scalar("SELECT count() FROM system.tables WHERE name LIKE '%\\\\_\\\\_tenant\\\\_%'") == 0


def test_the_dump_restores_the_original_after_backups_are_gone(warehouse: ClickHouse, tmp_path: Path) -> None:
    before = _fingerprint(warehouse)
    _apply(warehouse, _plan(warehouse), tmp_path)
    migration.finalize(warehouse)

    migration.restore(warehouse, tmp_path / "dump")

    assert _fingerprint(warehouse) == before
    assert _comment(warehouse, "bronze_example", "items") == "synthetic items"


def test_the_dump_restores_a_table_that_is_gone(warehouse: ClickHouse, tmp_path: Path) -> None:
    before = _fingerprint(warehouse)
    _apply(warehouse, _plan(warehouse), tmp_path)
    migration.finalize(warehouse)
    warehouse.execute("DROP TABLE staging.slack__users_snapshot")

    migration.restore(warehouse, tmp_path / "dump")

    assert _fingerprint(warehouse) == before


@dataclass(frozen=True)
class Interrupted(ClickHouse):
    """ClickHouse that loses one reply: the statement runs, the caller hears nothing.

    It fires at the first statement starting with `at`, or at the first one sent
    after a statement starting with `after`. Unless `once`, every statement after
    that fails too, as if the connection were gone.
    """

    at: str = ""
    after: str = ""
    once: bool = False
    seen: list[str] = field(default_factory=list)

    def execute(self, sql: str, *, fmt: str | None = None) -> str:
        statement = sql.lstrip()
        fired = "lost" in self.seen
        if fired and not self.once:
            raise ClickHouseError("connection lost")
        if not fired:
            if self.after and statement.startswith(self.after) and "armed" not in self.seen:
                self.seen.append("armed")
                return super().execute(sql, fmt=fmt)
            hit = statement.startswith(self.at) if self.at else "armed" in self.seen
            if hit:
                super().execute(sql, fmt=fmt)
                self.seen.append("lost")
                raise ClickHouseError("connection lost")
        return super().execute(sql, fmt=fmt)


def _interrupted(ch: ClickHouse, *, at: str = "", after: str = "", once: str = "") -> Interrupted:
    return Interrupted(url=ch.url, user=ch.user, password=ch.password, at=at, after=after, once=bool(once))


def _comment(ch: ClickHouse, database: str, name: str) -> str:
    rows = ch.rows(f"SELECT base64Encode(comment) FROM system.tables WHERE database = '{database}' AND name = '{name}'")
    return migration.decode_comment(rows[0][0]) if rows and rows[0] else ""


WITHOUT_COMMENT_IDS = [t for t in TABLES if t != "bronze_youtrack.youtrack_comments"]

BOUNDARIES = {
    "the swap's reply is lost": {"at": "EXCHANGE TABLES"},
    "the check after the swap fails": {"after": "EXCHANGE TABLES"},
    "the rename after the swap is lost": {"at": "RENAME TABLE"},
}


@pytest.mark.parametrize("where", BOUNDARIES.values(), ids=BOUNDARIES.keys())
def test_an_apply_cut_off_after_the_swap_is_rolled_back_by_a_later_run(
    warehouse: ClickHouse, tmp_path: Path, where: dict[str, str]
) -> None:
    before = _fingerprint(warehouse)
    plan = _plan(warehouse)

    with pytest.raises(migration.MigrationError, match="run `rollback`"):
        _apply(_interrupted(warehouse, **where), plan, tmp_path)
    migration.rollback(warehouse)
    migration.finalize(warehouse)

    after = _fingerprint(warehouse)
    assert {t: after[t] for t in WITHOUT_COMMENT_IDS} == {t: before[t] for t in WITHOUT_COMMENT_IDS}
    assert warehouse.scalar("SELECT count() FROM system.tables WHERE name LIKE '%\\_\\_tenant\\_%'") == 0


def test_an_apply_that_loses_one_reply_puts_the_original_back_itself(warehouse: ClickHouse, tmp_path: Path) -> None:
    before = _fingerprint(warehouse)
    once = _interrupted(warehouse, at="RENAME TABLE", once="yes")

    with pytest.raises(migration.MigrationError, match="The original is live again"):
        _apply(once, _plan(warehouse), tmp_path)

    after = _fingerprint(warehouse)
    assert {t: after[t] for t in WITHOUT_COMMENT_IDS} == {t: before[t] for t in WITHOUT_COMMENT_IDS}


def test_an_apply_cut_off_after_the_swap_can_be_kept_by_finalize(warehouse: ClickHouse, tmp_path: Path) -> None:
    with pytest.raises(migration.MigrationError):
        _apply(_interrupted(warehouse, at="RENAME TABLE"), _plan(warehouse), tmp_path)

    migration.finalize(warehouse)

    first = _first_migrated(warehouse)
    assert _column(warehouse, first, "tenant_id") == [NEW] * len(_column(warehouse, first, "tenant_id"))
    assert warehouse.scalar("SELECT count() FROM system.tables WHERE name LIKE '%\\_\\_tenant\\_%'") == 0


def _first_migrated(ch: ClickHouse) -> str:
    """The table an interrupted apply reached first: the first one with rows, in plan order."""
    return next(t.table.relation for t in _plan(ch).tables if t.rows)


def test_a_rollback_cut_off_mid_swap_is_finished_by_the_next_one(warehouse: ClickHouse, tmp_path: Path) -> None:
    before = _fingerprint(warehouse)
    _apply(warehouse, _plan(warehouse), tmp_path)

    with pytest.raises(ClickHouseError):
        migration.rollback(_interrupted(warehouse, at="EXCHANGE TABLES"))
    migration.rollback(warehouse)

    after = _fingerprint(warehouse)
    assert {t: after[t] for t in WITHOUT_COMMENT_IDS} == {t: before[t] for t in WITHOUT_COMMENT_IDS}


def test_a_restore_cut_off_mid_swap_is_finished_by_the_next_one(warehouse: ClickHouse, tmp_path: Path) -> None:
    before = _fingerprint(warehouse)
    _apply(warehouse, _plan(warehouse), tmp_path)
    migration.finalize(warehouse)

    with pytest.raises(ClickHouseError):
        migration.restore(_interrupted(warehouse, at="EXCHANGE TABLES"), tmp_path / "dump")
    migration.restore(warehouse, tmp_path / "dump")

    assert _fingerprint(warehouse) == before
    displaced = _column(warehouse, "bronze_bamboohr.whos_out__tenant_rejected", "unique_key")
    assert displaced == [f'["{NEW}","bamboo"]'], "the table the restore displaced is kept, not dropped"


@pytest.mark.parametrize(
    ("statement", "reason"),
    [
        (
            "INSERT INTO bronze_example.items VALUES ('other-main-1', 'other', 'main', '', now())",
            "holds another tenant",
        ),
        (
            f"INSERT INTO bronze_example.items VALUES ('{OLD}-elsewhere-1', '{OLD}', 'main', '', now())",
            "start like a moving tenant",
        ),
        (
            (
                "CREATE TABLE bronze_jira.jira_issue ENGINE = ReplacingMergeTree ORDER BY unique_key AS "
                f"SELECT '{OLD}-main-PROJ-1' AS unique_key, '{OLD}' AS tenant_id, 'main' AS source_id, '101' AS jira_id"
            ),
            "keyed by the issue key",
        ),
    ],
    ids=["second tenant", "ambiguous key", "jira heal pending"],
)
def test_a_refused_plan_changes_nothing(warehouse: ClickHouse, tmp_path: Path, statement: str, reason: str) -> None:
    warehouse.execute(statement)
    before = _fingerprint(warehouse)

    plan = _plan(warehouse)

    assert any(reason in r for r in plan.refusals), plan.refusals
    with pytest.raises(migration.MigrationError):
        _apply(warehouse, plan, tmp_path)
    assert _fingerprint(warehouse) == before
    assert not (tmp_path / "dump").exists()


def test_the_command_line_plans_applies_and_refuses_a_bad_tenant(warehouse: ClickHouse, tmp_path: Path) -> None:
    assert main(["plan", "--tenant", NEW, "--old-tenant", OLD]) == 0
    assert main(["apply", "--tenant", NEW.upper(), "--old-tenant", OLD, "--dump-dir", str(tmp_path / "x")]) == 1
    assert main(["apply", "--tenant", NEW, "--old-tenant", OLD, "--dump-dir", str(tmp_path / "dump")]) == 0
    assert (tmp_path / "dump" / "manifest.json").is_file()
    assert main(["finalize"]) == 0


TAB_AND_NEWLINE = "synthetic\titems\nsecond line"


def _set_items_comment(ch: ClickHouse, comment: str) -> None:
    escaped = comment.replace("\\", "\\\\").replace("'", "\\'")
    ch.execute(f"ALTER TABLE bronze_example.items MODIFY COMMENT '{escaped}'")


def _finalize(ch: ClickHouse, tmp_path: Path) -> None:
    migration.finalize(ch)


def _finalize_and_restore(ch: ClickHouse, tmp_path: Path) -> None:
    migration.finalize(ch)
    migration.restore(ch, tmp_path / "dump")


@pytest.mark.parametrize("settle", [_finalize, _finalize_and_restore], ids=["finalize", "restore"])
def test_a_comment_with_a_tab_and_a_newline_comes_back_whole(
    warehouse: ClickHouse, tmp_path: Path, settle: Callable[[ClickHouse, Path], None]
) -> None:
    _set_items_comment(warehouse, TAB_AND_NEWLINE)
    _apply(warehouse, _plan(warehouse), tmp_path)

    settle(warehouse, tmp_path)

    assert _comment(warehouse, "bronze_example", "items") == TAB_AND_NEWLINE


@dataclass(frozen=True)
class WithTableUuids(ClickHouse):
    """ClickHouse whose SHOW CREATE names each table's UUID, as a server-wide profile setting would."""

    def execute(self, sql: str, *, fmt: str | None = None) -> str:
        params = {"show_table_uuid_in_table_create_query_if_not_nil": "1"}
        if fmt:
            params["default_format"] = fmt
        with self._request(sql.encode("utf-8"), params=params) as response:
            return str(response.read().decode("utf-8"))


def test_a_dump_whose_ddl_names_the_table_uuid_still_restores(warehouse: ClickHouse, tmp_path: Path) -> None:
    before = _fingerprint(warehouse)
    with_uuids = WithTableUuids(url=warehouse.url, user=warehouse.user, password=warehouse.password)
    _apply(with_uuids, _plan(warehouse), tmp_path)
    assert " UUID '" in (tmp_path / "dump" / "bronze_example.items.sql").read_text()

    migration.restore(warehouse, tmp_path / "dump")

    assert _fingerprint(warehouse) == before
