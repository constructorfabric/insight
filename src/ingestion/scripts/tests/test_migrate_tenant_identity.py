"""Opt-in integration test: moving a warm install onto the canonical tenant.

`migrate-tenant-identity.sh` rewrites `tenant_id` in Bronze, config and the sync
ledger from a legacy key to the canonical tenant UUID, and must leave every
`unique_key` exactly as it was: the rows already stored and the rows connectors
write next both carry the legacy key segment, and ReplacingMergeTree collapses
them only while the keys agree.

Skipped unless a throwaway server is offered — the script rewrites every
`bronze_*` database it finds:

    docker run -d --rm --name ch-tenant -p 38211:8123 \\
        -e CLICKHOUSE_USER=insight -e CLICKHOUSE_PASSWORD=insight \\
        clickhouse/clickhouse-server:25.7.5
    TENANT_MIGRATION_TEST_CH_URL=http://localhost:38211 \\
    TENANT_MIGRATION_TEST_CH_USER=insight TENANT_MIGRATION_TEST_CH_PASSWORD=insight \\
        .venv/bin/python -m pytest tests/test_migrate_tenant_identity.py -q
"""

from __future__ import annotations

import os
import shutil
import subprocess
import urllib.request
from pathlib import Path

import pytest

CH_URL = os.environ.get("TENANT_MIGRATION_TEST_CH_URL")
CH_USER = os.environ.get("TENANT_MIGRATION_TEST_CH_USER", "default")
CH_PASSWORD = os.environ.get("TENANT_MIGRATION_TEST_CH_PASSWORD", "")

pytestmark = pytest.mark.skipif(not CH_URL, reason="set TENANT_MIGRATION_TEST_CH_URL to a throwaway server to run")

SCRIPTS = Path(__file__).resolve().parent.parent
MIGRATE = SCRIPTS / "migrate-tenant-identity.sh"
APPLY = SCRIPTS / "apply-ch-migrations.sh"

CANONICAL = "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"
LEGACY = "example-legacy"

SCHEMA = f"""
DROP DATABASE IF EXISTS bronze_example;
DROP DATABASE IF EXISTS config;
DROP DATABASE IF EXISTS ingestion_history;
DROP DATABASE IF EXISTS staging;
DROP DATABASE IF EXISTS bronze_youtrack;
CREATE DATABASE bronze_example;
CREATE DATABASE staging;
CREATE TABLE staging.jira__users_snapshot (unique_key String, tenant_id Nullable(String), _tracked_at DateTime)
  ENGINE = MergeTree ORDER BY tuple();
CREATE TABLE staging.claude_team__ai_overage (unique_key String, insight_tenant_id String)
  ENGINE = ReplacingMergeTree ORDER BY unique_key;
CREATE TABLE staging.jira__users_fields_history (unique_key String, tenant_id String)
  ENGINE = MergeTree ORDER BY tuple();
INSERT INTO staging.jira__users_snapshot VALUES ('{LEGACY}-main-u1', '{LEGACY}', now());
INSERT INTO staging.claude_team__ai_overage VALUES ('{LEGACY}-main-a1-2026-01', '{LEGACY}');
INSERT INTO staging.jira__users_fields_history VALUES ('{LEGACY}-main-u1', '{LEGACY}');
CREATE DATABASE config;
CREATE DATABASE ingestion_history;
CREATE TABLE bronze_example.events (unique_key String, tenant_id Nullable(String), source_id Nullable(String),
  v UInt32) ENGINE = ReplacingMergeTree ORDER BY unique_key;
CREATE TABLE bronze_example.no_tenant (unique_key String) ENGINE = ReplacingMergeTree ORDER BY unique_key;
CREATE TABLE config.value_map (tenant_id String, insight_source_id String, source_key String,
  unique_key String DEFAULT concat(tenant_id, ':', insight_source_id, ':', source_key))
  ENGINE = ReplacingMergeTree ORDER BY unique_key;
CREATE TABLE ingestion_history.sync_events (tenant_id LowCardinality(String), event_id String, ts DateTime)
  ENGINE = ReplacingMergeTree ORDER BY (tenant_id, event_id);
INSERT INTO bronze_example.events VALUES
  ('{LEGACY}-main-1', '{LEGACY}', 'main', 1),
  ('{LEGACY}-main-2', '{LEGACY}', 'main', 2);
INSERT INTO bronze_example.events VALUES ('{LEGACY}-main-2', '{CANONICAL}', 'main', 3);
INSERT INTO config.value_map (tenant_id, insight_source_id, source_key) VALUES ('{LEGACY}', 'main', 'k');
INSERT INTO ingestion_history.sync_events VALUES ('{LEGACY}', 'e1', now()), ('{LEGACY}', 'e2', now())
"""


def ch(sql: str) -> str:
    assert CH_URL and CH_URL.startswith(("http://", "https://")), CH_URL
    request = urllib.request.Request(
        f"{CH_URL}/",
        data=sql.encode(),
        headers={"X-ClickHouse-User": CH_USER, "X-ClickHouse-Key": CH_PASSWORD},
    )
    # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read().decode().strip()


def ch_script(sql: str) -> None:
    for statement in filter(str.strip, sql.split(";")):
        ch(statement)


def migrate(*args: str, legacy: str | None = LEGACY) -> subprocess.CompletedProcess[str]:
    env = {**os.environ, "CLICKHOUSE_URL": CH_URL, "CLICKHOUSE_USER": CH_USER, "CLICKHOUSE_PASSWORD": CH_PASSWORD}
    legacy_args = ["--legacy-tenant-key", legacy] if legacy else []
    return subprocess.run(
        [shutil.which("bash") or "bash", str(MIGRATE), "--canonical-tenant", CANONICAL, *legacy_args, *args],
        env=env, capture_output=True, text=True, check=False, timeout=300,
    )


def keys() -> str:
    return ch("SELECT arraySort(groupArray(unique_key)) FROM bronze_example.events")


@pytest.fixture(autouse=True)
def warm_install() -> None:
    ch_script(SCHEMA)


def test_tenant_columns_move_and_keys_stay() -> None:
    before = keys()

    result = migrate()

    assert result.returncode == 0, result.stderr
    assert keys() == before
    assert ch("SELECT groupUniqArray(tenant_id) FROM bronze_example.events") == f"['{CANONICAL}']"
    assert ch("SELECT count() FROM bronze_example.events FINAL") == "2"
    assert ch("SELECT tenant_id, unique_key FROM config.value_map FORMAT CSV") == f'"{CANONICAL}","{LEGACY}:main:k"'
    assert ch("SELECT groupUniqArray(toString(tenant_id)), count() FROM ingestion_history.sync_events FINAL") == (
        f"['{CANONICAL}']\t2"
    )
    assert "bronze_example.no_tenant" in result.stdout


def test_history_tables_move_and_derived_ones_are_left_to_the_full_refresh() -> None:
    assert migrate().returncode == 0
    assert ch("SELECT tenant_id, unique_key FROM staging.jira__users_snapshot FORMAT CSV") == (
        f'"{CANONICAL}","{LEGACY}-main-u1"'
    )
    assert ch("SELECT insight_tenant_id, unique_key FROM staging.claude_team__ai_overage FORMAT CSV") == (
        f'"{CANONICAL}","{LEGACY}-main-a1-2026-01"'
    )
    assert ch("SELECT tenant_id FROM staging.jira__users_fields_history") == LEGACY


def test_a_second_run_changes_nothing() -> None:
    assert migrate().returncode == 0
    second = migrate()
    assert second.returncode == 0, second.stderr
    assert f"rows moved to '{CANONICAL}': 0" in second.stdout


def test_a_row_without_a_tenant_moves_with_the_legacy_key() -> None:
    """On an install holding one tenant, a row that carries none can only be that tenant's."""
    ch("INSERT INTO bronze_example.events VALUES ('main-7', NULL, 'main', 7), ('main-8', '', 'main', 8)")

    result = migrate()

    assert result.returncode == 0, result.stderr
    assert ch("SELECT groupUniqArray(tenant_id) FROM bronze_example.events FINAL") == f"['{CANONICAL}']"


def test_without_a_legacy_key_only_rows_without_a_tenant_move() -> None:
    assert migrate().returncode == 0
    ch("INSERT INTO bronze_example.events VALUES ('main-7', NULL, 'main', 7)")

    result = migrate(legacy=None)

    assert result.returncode == 0, result.stderr
    assert ch("SELECT groupUniqArray(tenant_id), count() FROM bronze_example.events FINAL") == (
        f"['{CANONICAL}']\t3"
    )


def test_a_row_without_a_tenant_is_left_where_another_tenant_lives() -> None:
    ch("INSERT INTO bronze_example.events VALUES ('main-7', NULL, 'main', 7), ('other-main-9', 'other-tenant', 'main', 9)")

    assert migrate().returncode != 0
    assert ch("SELECT countIf(tenant_id IS NULL) FROM bronze_example.events") == "1"


def test_a_dry_run_changes_nothing() -> None:
    result = migrate("--dry-run")
    assert result.returncode == 0, result.stderr
    assert ch(f"SELECT countIf(tenant_id = '{LEGACY}') FROM bronze_example.events") == "2"


def test_an_install_holding_another_tenant_is_refused_untouched() -> None:
    ch("INSERT INTO bronze_example.events VALUES ('other-main-9', 'other-tenant', 'main', 9)")

    result = migrate()

    assert result.returncode != 0
    assert "other-tenant" in result.stderr
    assert ch(f"SELECT countIf(tenant_id = '{LEGACY}') FROM bronze_example.events") == "2"


@pytest.mark.parametrize(
    ("unique_key", "tenant_id", "rebuilt"),
    [
        (f"{LEGACY}-main-PROJ-7", CANONICAL, f"{LEGACY}-main-10007"),
        (f"{CANONICAL}-main-PROJ-7", CANONICAL, f"{CANONICAL}-main-10007"),
        ("unparseable", CANONICAL, f"{CANONICAL}-main-10007"),
    ],
)
def test_the_jira_rekey_keeps_the_key_tenant_segment(unique_key: str, tenant_id: str, rebuilt: str) -> None:
    expression = subprocess.run(
        ["bash", "-c", f"eval \"$(sed -n '/^_jira_key_tenant=/,/tenant_id)\"$/p' '{APPLY}')\"; printf %s \"$_jira_key_tenant\""],
        capture_output=True, text=True, check=True,
    ).stdout
    row = f"SELECT '{unique_key}' AS unique_key, '{tenant_id}' AS tenant_id, 'main' AS source_id, '10007' AS jira_id"
    assert ch(f"SELECT concat({expression}, '-', source_id, '-', jira_id) FROM ({row})") == rebuilt


def test_a_warm_key_on_the_issue_id_is_not_rebuilt_again() -> None:
    row = f"SELECT '{LEGACY}-main-10007' AS unique_key, '{CANONICAL}' AS tenant_id, 'main' AS source_id, '10007' AS jira_id"
    keyed_by_issue_key = ch(
        f"SELECT NOT endsWith(coalesce(unique_key, ''), concat('-', coalesce(source_id, ''), '-', coalesce(jira_id, '')))"
        f" FROM ({row})"
    )
    assert keyed_by_issue_key == "0"


YOUTRACK_OBSERVATIONS = f"""
CREATE DATABASE bronze_youtrack;
CREATE TABLE bronze_youtrack.youtrack_custom_fields (unique_key String, tenant_id Nullable(String), source_id Nullable(String),
  id Nullable(String)) ENGINE = ReplacingMergeTree ORDER BY unique_key;
INSERT INTO bronze_youtrack.youtrack_custom_fields VALUES ('{LEGACY}-yt-main-f1', '{LEGACY}', 'yt-main', 'f1');
CREATE TABLE staging.youtrack__field_observations (unique_key String, insight_source_id String, field_id String)
  ENGINE = ReplacingMergeTree ORDER BY unique_key;
INSERT INTO staging.youtrack__field_observations VALUES ('{LEGACY}-yt-main-f1-2026-01-05 00:00:00', 'yt-main', 'f1');
INSERT INTO staging.youtrack__field_observations VALUES ('other-yt-main-f2-2026-01-05 00:00:00', 'yt-main', 'f2')
"""


def heal_field_observations() -> subprocess.CompletedProcess[str]:
    functions = r"sed -n '/^ch_table_is_real() {/,/^}/p; /^heal_youtrack_field_observation_tenant() {/,/^}/p'"
    env = {**os.environ, "CLICKHOUSE_URL": CH_URL, "CLICKHOUSE_USER": CH_USER, "CLICKHOUSE_PASSWORD": CH_PASSWORD}
    script = (
        f"set -euo pipefail; source '{SCRIPTS / 'lib/ch-exec.sh'}'; eval \"$({functions} '{APPLY}')\";"
        " heal_youtrack_field_observation_tenant; heal_youtrack_field_observation_tenant"
    )
    return subprocess.run(
        [shutil.which("bash") or "bash", "-c", script],
        env=env, capture_output=True, text=True, check=False, timeout=120,
    )


def observations() -> list[str]:
    return ch(
        "SELECT field_id, ifNull(tenant_id, 'NULL'), unique_key FROM staging.youtrack__field_observations"
        " ORDER BY field_id FORMAT TSV"
    ).splitlines()


def test_an_observation_takes_the_tenant_its_key_was_written_under() -> None:
    """The heal reads the tenant back from the key and leaves a key naming no tenant Bronze
    holds unattributed; the transition then moves both onto the install's one tenant."""
    ch_script(YOUTRACK_OBSERVATIONS)

    healed = heal_field_observations()

    assert healed.returncode == 0, healed.stderr
    assert observations() == [
        f"f1\t{LEGACY}\t{LEGACY}-yt-main-f1-2026-01-05 00:00:00",
        "f2\tNULL\tother-yt-main-f2-2026-01-05 00:00:00",
    ]

    assert migrate().returncode == 0
    assert observations() == [
        f"f1\t{CANONICAL}\t{LEGACY}-yt-main-f1-2026-01-05 00:00:00",
        f"f2\t{CANONICAL}\tother-yt-main-f2-2026-01-05 00:00:00",
    ]
