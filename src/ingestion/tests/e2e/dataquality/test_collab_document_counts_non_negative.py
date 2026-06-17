"""PoC: run a data-quality catalog check against seeded silver data.

Demonstrates the pattern requested in #1348 — wiring one `data_quality` check
into the e2e rig with a good/bad fixture pair, so the check is itself tested:

  * seed the silver table the check guards with KNOWN-GOOD rows  -> check passes
  * seed it with a KNOWN-BAD row (a negative count)              -> check fires

The check under test is `assert_collab_document_counts_non_negative` (#1321 /
PR #1350): activity counts can never be negative. It is `severity='warn'`, so
dbt exits 0 either way — we assert on the `failures` count (violating rows)
read from run_results.json, exactly as the deployed data-quality emitter does.

We seed the SILVER table directly rather than building from bronze: the check
operates on silver, and the bronze `sharepoint_activity` placeholder does not
yet carry the document-activity source columns (a separate gap noted in the PR).
The silver placeholder was aligned to the model (unique_key / synced_count /
visited_page_count added) so the check's projection resolves.
"""

from __future__ import annotations

import datetime as dt

import pytest

from e2e_lib import clickhouse as ch
from e2e_lib.config import SessionConfig
from e2e_lib.dbt_runner import DbtRunner
from e2e_lib.worker import WorkerContext

pytestmark = pytest.mark.smoke

CHECK = "assert_collab_document_counts_non_negative"
TABLE = "class_collab_document_activity"
COLS = [
    "insight_tenant_id",
    "email",
    "person_key",
    "date",
    "data_source",
    "unique_key",
    "product",
    "shared_internally_count",
    "shared_externally_count",
    "viewed_or_edited_count",
    "synced_count",
    "visited_page_count",
    "_version",
]
_NIL_TENANT = "00000000-0000-0000-0000-000000000000"


def _row(unique_key: str, viewed: float) -> tuple:
    return (
        _NIL_TENANT,
        "alice@example.com",
        "alice@example.com",
        dt.date(2026, 6, 1),
        "insight_m365",
        unique_key,
        "sharepoint",
        1.0,  # shared_internally_count
        0.0,  # shared_externally_count
        viewed,  # viewed_or_edited_count — the column we flip negative
        2.0,  # synced_count
        5.0,  # visited_page_count
        1,  # _version
    )


def _seed(cfg: SessionConfig, rows: list[tuple]) -> None:
    """Replace the silver table contents with exactly `rows`."""
    ch.execute(cfg, f"TRUNCATE TABLE IF EXISTS silver.{TABLE}")
    with ch.client(cfg, database="silver") as c:
        c.insert(table=TABLE, data=rows, column_names=COLS)


def test_clean_data_passes(
    session_cfg: SessionConfig,
    dbt_runner: DbtRunner,
    worker_ctx: WorkerContext,
) -> None:
    """Known-good rows (all counts >= 0) -> the check reports zero violations."""
    _seed(session_cfg, [_row("t-m365-alice-2026-06-01-sharepoint", viewed=10.0)])
    status, failures = dbt_runner.run_test(CHECK, worker_ctx=worker_ctx)
    assert failures == 0, f"expected 0 violations on clean data, got {failures}"
    assert status == "pass", f"expected status 'pass', got {status!r}"


def test_negative_count_is_flagged(
    session_cfg: SessionConfig,
    dbt_runner: DbtRunner,
    worker_ctx: WorkerContext,
) -> None:
    """One negative count among good rows -> the check flags exactly that row,
    and as a warn-severity check it surfaces a finding without erroring."""
    _seed(
        session_cfg,
        [
            _row("t-m365-alice-2026-06-01-sharepoint", viewed=10.0),
            _row("t-m365-bob-2026-06-01-sharepoint", viewed=-3.0),
        ],
    )
    status, failures = dbt_runner.run_test(CHECK, worker_ctx=worker_ctx)
    assert failures == 1, f"expected exactly 1 flagged row, got {failures}"
    # severity='warn' → the violation is a finding, not a hard error: it must
    # NOT fail the build. This is the behaviour PR #1350 deliberately chose.
    assert status == "warn", f"expected status 'warn' (non-blocking), got {status!r}"
