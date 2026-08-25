"""Empty the ClickHouse relations that hold fixture data, once per session.

A session must start from empty bronze/staging/silver. The staging and silver
models are incremental behind a watermark over their own target
(`max(_airbyte_extracted_at)`, `max(_version)`), and a fixture pins the value it
seeds — so against a ClickHouse that already holds a previous session's rows the
watermark admits nothing and the fixture's assertions read that older data.

The per-test truncate ledger (`lib.ch_seeder`) covers only the tables a fixture
seeds and the models dbt rebuilds for it, and it is created empty per session, so
it cannot carry isolation across sessions. This module does.
"""

from __future__ import annotations

import logging

from lib import clickhouse as ch
from lib.config import SessionConfig

LOG = logging.getLogger("e2e.reset")

# Every relation in these databases is fixture data: seeded bronze, plus the
# staging and silver relations dbt derives from it. Migrations and the
# connectors-ddl snapshot create structure only, so nothing here outlives a
# session. The gold database is absent because every model serving it is
# `materialized='table'` and the `tag:gold` build each test runs replaces those
# wholesale; `identity` is absent because the metrics and identity suites each
# truncate it per test.
DATA_DATABASE_PATTERN = "^(bronze_.*|staging|silver)$"


def truncate_data_tables(cfg: SessionConfig) -> int:
    """TRUNCATE every fixture-data table, returning how many were truncated.

    Only the MergeTree family is matched: dbt materializes several staging models
    as views, which hold no rows of their own and which ClickHouse refuses to
    truncate (NOT_IMPLEMENTED, code 48). A relation outside that family that does
    hold rows is a gap — `meta/test_session_reset.py` fails on it rather than
    letting it silently survive.
    """
    tables = ch.query(
        cfg,
        f"""
        SELECT database, name
        FROM system.tables
        WHERE match(database, '{DATA_DATABASE_PATTERN}')
          AND engine LIKE '%MergeTree'
        ORDER BY database, name
        """,
    )
    for database, table in tables:
        ch.execute(cfg, f"TRUNCATE TABLE `{database}`.`{table}`")
    LOG.info("session-start reset: truncated %d bronze/staging/silver tables", len(tables))
    return len(tables)
