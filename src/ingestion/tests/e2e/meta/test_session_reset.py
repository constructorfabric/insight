"""Guards for the session-start data reset (`lib.session_reset`).

The reset is what keeps a fixture's assertions on its own seed: without it, the
incremental staging and silver models admit nothing from a fixture that re-seeds
timestamps a previous session already loaded, and the fixture reads that older
data instead.
"""

from __future__ import annotations

import pytest
from lib import clickhouse, session_reset
from lib.config import SessionConfig

pytestmark = pytest.mark.smoke


def test_reset_covers_the_bronze_staging_silver_relations(ch_migrations_applied: SessionConfig) -> None:
    """No bronze, staging or silver relation holds rows once the reset has run.

    The reset runs here rather than being taken on trust from session setup, so
    the guard holds whatever order the suite collects in. A relation still
    holding rows is one the engine filter in `truncate_data_tables` misses.
    """
    cfg = ch_migrations_applied

    truncated = session_reset.truncate_data_tables(cfg)
    assert truncated > 0, "the reset matched no relations — check DATA_DATABASE_PATTERN"

    holding_rows = clickhouse.query(
        cfg,
        f"""
        SELECT database, name, total_rows
        FROM system.tables
        WHERE match(database, '{session_reset.DATA_DATABASE_PATTERN}')
          AND total_rows > 0
        ORDER BY database, name
        """,
    )
    assert holding_rows == [], f"relations still holding rows after the reset: {holding_rows}"
