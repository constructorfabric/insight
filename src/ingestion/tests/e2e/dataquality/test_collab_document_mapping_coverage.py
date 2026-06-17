"""PoC: run the mapping-coverage drift/floor check against seeded silver (#736).

Companion to test_collab_document_counts_non_negative — same pattern, but for a
*cross-table referential* check: activity.person_key (= lower(email)) must map to
a BambooHR class_people email. The check
(`assert_collab_document_activity_mapping_coverage`) flags the latest complete day
per tenant when mapped-coverage drops >5pt vs its 7-day trailing average OR falls
under the 75% floor.

We seed both silver tables directly (the check reads silver):
  * a BambooHR roster of two people (alice, bob),
  * a multi-day activity series — healthy days where every active user maps, and
    (in the bad case) a latest day of all-orphan activity.

Dates are relative (today - N) so the check's `date < today()` and trailing
window behave; the incomplete current day is never seeded.
"""

from __future__ import annotations

import datetime as dt

import pytest

from e2e_lib import clickhouse as ch
from e2e_lib.config import SessionConfig
from e2e_lib.dbt_runner import DbtRunner
from e2e_lib.worker import WorkerContext

pytestmark = pytest.mark.smoke

CHECK = "assert_collab_document_activity_mapping_coverage"
TENANT = "00000000-0000-0000-0000-0000aaaa0001"

_PEOPLE_COLS = ["unique_key", "tenant_id", "source", "email", "_version"]
_ACT_COLS = ["tenant_id", "person_key", "date", "_version"]

_ROSTER = [
    ("pk-alice", TENANT, "bamboohr", "alice@example.com", 1),
    ("pk-bob", TENANT, "bamboohr", "bob@example.com", 1),
]
_MAPPED = ["alice@example.com", "bob@example.com"]  # match the roster
_ORPHAN = ["carol@external.com", "dave@external.com"]  # no roster match


def _seed_roster(cfg: SessionConfig) -> None:
    ch.execute(cfg, "TRUNCATE TABLE IF EXISTS silver.class_people")
    with ch.client(cfg, database="silver") as c:
        c.insert(table="class_people", data=_ROSTER, column_names=_PEOPLE_COLS)


def _seed_activity(cfg: SessionConfig, day_to_keys: dict[int, list[str]]) -> None:
    """Seed activity rows. `day_to_keys` maps days-ago -> the person_keys active
    that day. Each (day, person_key) is one row."""
    ch.execute(cfg, "TRUNCATE TABLE IF EXISTS silver.class_collab_document_activity")
    rows = []
    for days_ago, keys in day_to_keys.items():
        event_date = dt.date.today() - dt.timedelta(days=days_ago)
        for key in keys:
            rows.append((TENANT, key, event_date, 1))
    with ch.client(cfg, database="silver") as c:
        c.insert(
            table="class_collab_document_activity", data=rows, column_names=_ACT_COLS
        )


def test_healthy_coverage_passes(
    session_cfg: SessionConfig,
    dbt_runner: DbtRunner,
    worker_ctx: WorkerContext,
) -> None:
    """8 days where every active user maps to the roster -> coverage 1.0
    throughout, no drift, above floor -> zero violations."""
    _seed_roster(session_cfg)
    _seed_activity(session_cfg, {d: _MAPPED for d in range(1, 9)})
    status, failures = dbt_runner.run_test(CHECK, worker_ctx=worker_ctx)
    assert failures == 0, f"healthy coverage should not flag, got {failures}"
    assert status == "pass", f"expected status 'pass', got {status!r}"


def test_coverage_collapse_is_flagged(
    session_cfg: SessionConfig,
    dbt_runner: DbtRunner,
    worker_ctx: WorkerContext,
) -> None:
    """7 healthy days then a latest day of all-orphan activity -> latest coverage
    0.0 trips BOTH the 75% floor and the >5pt drift vs the trailing 1.0 baseline.
    Exactly the one latest tenant-day is flagged; warn-severity (non-blocking)."""
    _seed_roster(session_cfg)
    series = {d: _MAPPED for d in range(2, 9)}  # days 2..8 healthy (7 prior days)
    series[1] = _ORPHAN  # latest complete day (today-1): all unmapped
    _seed_activity(session_cfg, series)
    status, failures = dbt_runner.run_test(CHECK, worker_ctx=worker_ctx)
    assert failures == 1, (
        f"expected exactly the latest tenant-day flagged, got {failures}"
    )
    assert status == "warn", f"expected status 'warn' (non-blocking), got {status!r}"
