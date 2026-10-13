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
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from tenant_migration import apply as migration
from tenant_migration.__main__ import main
from tenant_migration.clickhouse import ClickHouse
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
ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
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
    for database in DATABASES:
        ch.execute(f"DROP DATABASE IF EXISTS {database} SYNC")
        ch.execute(f"CREATE DATABASE {database}")
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


def test_a_second_run_finds_nothing_to_move(warehouse: ClickHouse, tmp_path: Path) -> None:
    _apply(warehouse, _plan(warehouse), tmp_path)

    plan = _plan(warehouse)

    assert plan.changing == [] and plan.refusals == [], plan.refusals


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
