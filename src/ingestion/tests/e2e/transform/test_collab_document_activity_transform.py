"""Transform test for `silver.class_collab_document_activity` (bronze → silver).

This is the sibling the #1348 data-quality PoC could not be: that test seeds
SILVER directly and proves "bad data in silver → the check fires". It does NOT
prove the bronze→silver transform is correct — if a staging bug silently
dropped or clamped a negative count, silver would never see it and the check
would pass against production-broken data.

This test closes that blind spot. It seeds the **bronze** M365 document-activity
sources, runs `dbt build --select +class_collab_document_activity` so the real
staging models execute, then reads the resulting silver rows and asserts the
transform's contract:

  * SharePoint and OneDrive halves both land and are tagged by `product`
  * `email` keeps source casing; `person_key` is lower-cased  (the lower() rule)
  * `reportRefreshDate` (a String in bronze) becomes a real `date`
  * OneDrive's `visited_page_count` is NULL; SharePoint's carries through
  * `unique_key` is the documented MD5 of (tenant, source, upn, date, product)
  * a NEGATIVE `viewedOrEditedFileCount` is PRESERVED through the transform —
    the exact case the silver-seed PoC had to assume rather than prove

Seeds bronze and TRUNCATEs the staging+silver relations first so the incremental
models take their full-reprocess branch deterministically (same serial, shared-
table model as the rest of the rig — see conftest.py).
"""

from __future__ import annotations

import datetime as dt
import hashlib

import pytest

from e2e_lib import clickhouse as ch
from e2e_lib.config import SessionConfig
from e2e_lib.dbt_runner import DbtRunner
from e2e_lib.worker import WorkerContext

pytestmark = pytest.mark.smoke

SELECTOR = "+class_collab_document_activity"

_TENANT = "11111111-1111-1111-1111-111111111111"
_SOURCE = "22222222-2222-2222-2222-222222222222"

# Relative date, never hardcoded: the staging models filter recent extracts and
# `toDate(reportRefreshDate)` flows into silver — a stale 2026 literal could age
# out of any date-bounded sibling logic. See the #1348 PoC for the same rule.
_REFRESH = (dt.date.today() - dt.timedelta(days=1)).isoformat()

# Relations the build writes — TRUNCATEd up front so incremental models reprocess
# every seeded row (empty target → `(SELECT count() FROM this) = 0` branch).
_SILVER = "silver.class_collab_document_activity"
_STG_SP = "staging.m365__collab_document_activity_sharepoint"
_STG_OD = "staging.m365__collab_document_activity_onedrive"
_BRONZE_SP = "bronze_m365.sharepoint_activity"
_BRONZE_OD = "bronze_m365.onedrive_activity"

_SP_COLS = [
    "tenant_id", "source_id", "userPrincipalName", "lastActivityDate",
    "reportRefreshDate", "reportPeriod", "viewedOrEditedFileCount",
    "syncedFileCount", "sharedInternallyFileCount", "sharedExternallyFileCount",
    "visitedPageCount",
]
_OD_COLS = [
    "tenant_id", "source_id", "userPrincipalName", "lastActivityDate",
    "reportRefreshDate", "reportPeriod", "viewedOrEditedFileCount",
    "syncedFileCount", "sharedInternallyFileCount", "sharedExternallyFileCount",
]


def _expected_unique_key(upn: str, product: str) -> str:
    """Mirror the staging model's
    MD5(concat(tenant, '-', source, '-', coalesce(upn,''), '-', toString(date), '-', product)).
    """
    raw = f"{_TENANT}-{_SOURCE}-{upn}-{_REFRESH}-{product}"
    return hashlib.md5(raw.encode()).hexdigest()


def _seed(cfg: SessionConfig) -> None:
    # DROP (not TRUNCATE) the silver + staging relations: the silver target ships
    # as an init.sh placeholder whose drifted schema has no `unique_key`, so the
    # incremental delete+insert strategy errors ("Missing columns: unique_key").
    # Dropping forces dbt to full-refresh both with the real model schema. Bronze
    # tables are the sources the staging models read, so keep them — just clear.
    for rel in (_SILVER, _STG_SP, _STG_OD):
        ch.execute(cfg, f"DROP TABLE IF EXISTS {rel}")
    for rel in (_BRONZE_SP, _BRONZE_OD):
        ch.execute(cfg, f"TRUNCATE TABLE IF EXISTS {rel}")

    # SharePoint: one healthy row + one with a NEGATIVE viewed count.
    # Trailing value is visitedPageCount (Int64); the rest are Float64 counts.
    sp_rows = [
        (_TENANT, _SOURCE, "Alice@Example.com", _REFRESH, _REFRESH, "7",
         10.0, 2.0, 1.0, 0.0, 5),
        (_TENANT, _SOURCE, "bob@example.com", _REFRESH, _REFRESH, "7",
         -3.0, 0.0, 0.0, 0.0, 0),
    ]
    # OneDrive: one row for the same person (no visitedPageCount in this source).
    od_rows = [
        (_TENANT, _SOURCE, "Alice@Example.com", _REFRESH, _REFRESH, "7",
         4.0, 1.0, 2.0, 1.0),
    ]
    with ch.client(cfg, database="bronze_m365") as c:
        c.insert(table="sharepoint_activity", data=sp_rows, column_names=_SP_COLS)
        c.insert(table="onedrive_activity", data=od_rows, column_names=_OD_COLS)


def test_bronze_to_silver_document_activity(
    session_cfg: SessionConfig,
    dbt_runner: DbtRunner,
    worker_ctx: WorkerContext,
) -> None:
    _seed(session_cfg)
    dbt_runner.build(SELECTOR, worker_ctx=worker_ctx)

    rows = ch.query(
        session_cfg,
        f"""
        SELECT product,
               email,
               person_key,
               toString(date)            AS date,
               viewed_or_edited_count,
               synced_count,
               shared_internally_count,
               shared_externally_count,
               visited_page_count,
               report_period,
               data_source,
               lower(hex(unique_key))    AS uk
        FROM {_SILVER}
        ORDER BY product, email
        """,
    )

    # 2 SharePoint + 1 OneDrive, nothing dropped.
    assert len(rows) == 3, f"expected 3 silver rows, got {len(rows)}: {rows}"

    by_key = {(r[0], r[1]): r for r in rows}
    sp_alice = by_key[("sharepoint", "Alice@Example.com")]
    sp_bob = by_key[("sharepoint", "bob@example.com")]
    od_alice = by_key[("onedrive", "Alice@Example.com")]

    # email keeps source casing; person_key is lower-cased (the lower() rule).
    assert sp_alice[2] == "alice@example.com"
    assert od_alice[2] == "alice@example.com"

    # reportRefreshDate (String) → real date.
    assert sp_alice[3] == _REFRESH

    # The point of this test: a negative count survives the transform intact.
    assert sp_bob[4] == -3.0, f"negative viewed_or_edited_count not preserved: {sp_bob[4]}"

    # SharePoint carries visited_page_count; OneDrive nulls it.
    assert sp_alice[8] == 5.0
    assert od_alice[8] is None, f"OneDrive visited_page_count should be NULL, got {od_alice[8]}"

    # Straight column mapping for the healthy SharePoint row.
    assert (sp_alice[5], sp_alice[6], sp_alice[7]) == (2.0, 1.0, 0.0)
    assert sp_alice[9] == "7"
    assert sp_alice[10] == "insight_m365"

    # unique_key is the documented MD5 over (tenant, source, upn, date, product).
    assert sp_alice[11] == _expected_unique_key("Alice@Example.com", "sharepoint")
    assert od_alice[11] == _expected_unique_key("Alice@Example.com", "onedrive")
