"""Integration tests for ch-seeder — require compose to be up.

These exercise the typed INSERT path against a real ClickHouse, validating
that:
  - String / integer / Date / Nullable columns are coerced correctly
  - empty cells become SQL NULL (not the literal string "")
  - the touched-tables ledger drives selective TRUNCATE
"""

from __future__ import annotations

from pathlib import Path

import pandas as pd
import pytest

from e2e_lib import clickhouse as ch
from e2e_lib.ch_seeder import CHSeeder, SeederError
from e2e_lib.config import SessionConfig
from e2e_lib.fixture_loader import BronzeCsv, Fixture, SpecYaml


pytestmark = pytest.mark.smoke


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(scope="module")
def seeder_table(ch_migrations_applied: SessionConfig) -> tuple[str, str]:
    """Module-scoped: create a bronze test table once for all ch-seeder tests."""
    cfg = ch_migrations_applied
    schema = "bronze_e2e_test"
    table = "events"
    ch.execute(cfg, f"CREATE DATABASE IF NOT EXISTS {schema}")
    ch.execute(cfg, f"DROP TABLE IF EXISTS {schema}.{table}")
    ch.execute(
        cfg,
        f"""
        CREATE TABLE {schema}.{table} (
            id           UInt64,
            actor_name   Nullable(String),
            event_date   Nullable(Date),
            score        Nullable(Float64),
            tags         Nullable(String)
        )
        ENGINE = MergeTree
        ORDER BY id
        """,
    )
    yield schema, table
    ch.execute(cfg, f"DROP TABLE IF EXISTS {schema}.{table}")
    ch.execute(cfg, f"DROP DATABASE IF EXISTS {schema}")


def _make_fixture(tmp_path: Path, csv_body: str, schema: str, table: str) -> Fixture:
    bronze_dir = tmp_path / "bronze"
    bronze_dir.mkdir(parents=True, exist_ok=True)
    csv_path = bronze_dir / f"{schema}.{table}.csv"
    csv_path.write_text(csv_body)

    return Fixture(
        name="seed_test",
        root=tmp_path,
        spec=SpecYaml(
            spec_version=1,
            endpoint="/v1/metrics/{metric_id}/query",
            request_body={},
            dbt_selector="+silver_test+",
            key_columns=["id"],
            method="POST",
            metric_id="00000000-0000-0000-0000-000000000001",
        ),
        bronze_csvs=[BronzeCsv(path=csv_path, schema=schema, table=table)],
    )


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


def test_seed_typed_columns(
    ch_migrations_applied: SessionConfig,
    seeder_table: tuple[str, str],
    tmp_path: Path,
) -> None:
    """All three row types — string, int, date — round-trip correctly. Empty cell = NULL."""
    schema, table = seeder_table
    cfg = ch_migrations_applied
    seeder = CHSeeder(cfg)

    csv = (
        "id,actor_name,event_date,score,tags\n"
        "1,Alice,2026-01-15,4.5,foo\n"
        "2,Bob,2026-01-16,,bar\n"  # score empty → NULL
        "3,,2026-01-17,3.0,\n"     # actor_name AND tags empty
    )
    seeder.seed(_make_fixture(tmp_path, csv, schema, table))

    rows = ch.query(cfg, f"SELECT id, actor_name, toString(event_date), score, tags FROM {schema}.{table} ORDER BY id")
    assert rows == [
        (1, "Alice", "2026-01-15", 4.5, "foo"),
        (2, "Bob", "2026-01-16", None, "bar"),
        (3, None, "2026-01-17", 3.0, None),
    ]


def test_seed_unknown_table_raises(
    ch_migrations_applied: SessionConfig,
    tmp_path: Path,
) -> None:
    """Targeting a non-existent bronze table fails fast with a useful message."""
    cfg = ch_migrations_applied
    seeder = CHSeeder(cfg)
    fx = _make_fixture(tmp_path, "id\n1\n", schema="bronze_does_not_exist", table="nope")
    with pytest.raises(SeederError, match="not found in system.columns"):
        seeder.seed(fx)


def test_seed_unknown_column_raises(
    ch_migrations_applied: SessionConfig,
    seeder_table: tuple[str, str],
    tmp_path: Path,
) -> None:
    schema, table = seeder_table
    cfg = ch_migrations_applied
    seeder = CHSeeder(cfg)
    fx = _make_fixture(tmp_path, "id,bogus_column\n42,boom\n", schema, table)
    with pytest.raises(SeederError, match="columns not in"):
        seeder.seed(fx)


def test_truncate_ledger(
    ch_migrations_applied: SessionConfig,
    seeder_table: tuple[str, str],
    tmp_path: Path,
) -> None:
    """seed() records touched tables; truncate_touched() empties only those."""
    schema, table = seeder_table
    cfg = ch_migrations_applied
    seeder = CHSeeder(cfg)

    # Pre-clean to guarantee a known starting state (the module-scoped table
    # may have rows left from earlier tests in the same session).
    ch.execute(cfg, f"TRUNCATE TABLE {schema}.{table}")

    seeder.seed(_make_fixture(tmp_path, "id\n100\n", schema, table))
    pre = ch.query(cfg, f"SELECT count() FROM {schema}.{table}")
    assert pre == [(1,)]

    truncated_n = seeder.truncate_touched()
    assert truncated_n == 1

    post = ch.query(cfg, f"SELECT count() FROM {schema}.{table}")
    assert post == [(0,)]

    # Second call is a no-op (ledger drained)
    assert seeder.truncate_touched() == 0
