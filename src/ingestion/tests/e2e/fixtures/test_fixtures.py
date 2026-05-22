"""Parametrized fixture-runner.

One pytest invocation per `fixtures/<name>/` folder. The body wires together
every component built in DECOMPOSITION:

    truncate-touched-bronze  →
    seed bronze CSVs          →
    dbt build --select        →
    POST <endpoint>           →
    pandas diff vs expected/response.csv

The collection hook is in `../conftest.py`; this file only contains the test
body and per-test fixtures (function-scoped).
"""

from __future__ import annotations

import logging
import os
import pytest

from e2e_lib.analytics_api import AnalyticsApiProcess
from e2e_lib.ch_seeder import CHSeeder
from e2e_lib.csv_asserter import assert_matches, update_snapshot
from e2e_lib.dbt_runner import DbtRunner
from e2e_lib.fixture_loader import Fixture
from e2e_lib.migration_applier import refresh_intermediates
from e2e_lib.worker import WorkerContext


pytestmark = pytest.mark.fixture
LOG = logging.getLogger("e2e.runner")


def test_fixture(
    fixture: Fixture,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    analytics_api: AnalyticsApiProcess,
    worker_ctx: WorkerContext,
    update_snapshots: bool,
) -> None:
    # Step 1: clear what the prior test wrote (no-op on first test of session)
    ch_seeder.truncate_touched()

    # Step 2: seed the bronze AND silver tables this fixture targets
    ch_seeder.seed(fixture)

    # Step 3: run only the silver/staging models this fixture needs.
    # Some fixtures read view-only metrics (e.g. insight.people) — they
    # don't need any dbt model and omit dbt_selector entirely.
    if fixture.spec.dbt_selector:
        dbt_runner.build(fixture.spec.dbt_selector, worker_ctx=worker_ctx)

    # Step 3b: refresh materialized intermediates (task_issue_current_state etc).
    # These are MVs with a 1-hour refresh schedule in prod; we trigger sync now
    # so the fixture's silver writes are visible to gold views immediately.
    # Cheap (~tens of ms on a fresh CH) — always-on is simpler than gating.
    refresh_intermediates(ch_seeder.cfg)

    # Step 4: call the API
    response = analytics_api.call_fixture(fixture)
    if response.status_code != 200:
        LOG.warning("API returned %d; body: %r", response.status_code, response.raw)

    # Step 5: assert OR snapshot-update
    if update_snapshots:
        if os.environ.get("CI") == "true":
            pytest.fail("--update-snapshots is forbidden under CI=true")
        # Refuse to write an empty/error-body snapshot — that would silently
        # encode the broken state and make the NEXT run "pass" against a
        # garbage expectation. The author must fix the API call first.
        if response.status_code != 200:
            pytest.fail(
                f"--update-snapshots refused: API returned {response.status_code}, "
                f"not 200. Fix the request/state first.\n"
                f"  body: {response.raw!r}"
            )
        summary = update_snapshot(response, fixture)
        target = fixture.root / "expected" / "response.csv"
        LOG.info("snapshot updated: %s", target)
        if summary and summary != "(no changes)":
            LOG.info("changes:\n%s", summary)
        return
    assert_matches(response, fixture)
