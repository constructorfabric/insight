"""Parametrized runner for `*.test.yaml` fixtures.

One pytest invocation per discovered `<name>.test.yaml`. Implements
`cpt-bronze-to-api-e2e-algo-yaml-execute-test`:

    truncate prior test's tables  →
    seed resolved bronze records  →
    two-pass dbt build (staging, then silver)  →
    recreate gold views (reapply migrations)   →
    refresh intermediates  →
    POST /v1/metrics/queries per case  →
    evaluate expect rules

Discovery + per-test fixtures live in `../conftest.py`.
"""

from __future__ import annotations

import logging

import pytest

from e2e_lib.analytics_api import AnalyticsApiProcess
from e2e_lib.ch_seeder import CHSeeder
from e2e_lib.dbt_runner import DbtRunner
from e2e_lib.expect_engine import evaluate_case
from e2e_lib.fixture_loader import TestYaml
from e2e_lib.migration_applier import refresh_intermediates, reapply_migrations
from e2e_lib.worker import WorkerContext

pytestmark = pytest.mark.fixture
LOG = logging.getLogger("e2e.runner")


def test_e2e_metric_smoke(
    test_yaml: TestYaml,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    analytics_api: AnalyticsApiProcess,
    worker_ctx: WorkerContext,
) -> None:
    # 1. Clear what the prior test wrote (no-op on the first test).
    ch_seeder.truncate_touched()

    # 2. Seed this test's resolved bronze records.
    ch_seeder.seed_bronze(test_yaml.bronze)

    # 3. Build the dbt models the seeded tables feed: staging first (the `+`
    #    pulls <connector>__bronze_promoted), then the silver class models.
    staging, silver = dbt_runner.derive_selectors(test_yaml.touched_tables)
    if staging:
        # Record staging models in the ledger BEFORE building. They live in the
        # `staging` schema and are read by the silver models via union_by_tag, so a
        # prior test's staging rows (e.g. dates this test doesn't re-seed) would
        # survive into the silver rebuild and contaminate later tests' gold-view
        # aggregates. Recording up front (not after) means a build that raises
        # partway still leaves the table in the truncate ledger so the next test
        # cleans it; recording a model that never materialised is harmless
        # (truncate_touched uses TRUNCATE TABLE IF EXISTS).
        for st in staging:
            ch_seeder.ledger.record("staging", st)
        dbt_runner.build(" ".join(f"+{m}" for m in staging), worker_ctx=worker_ctx)
    if silver:
        for cls in silver:
            ch_seeder.ledger.record("silver", cls)
        dbt_runner.build(" ".join(silver), worker_ctx=worker_ctx)

    # 4. Recreate gold views against the now-real silver schema (fixes the rig-only
    #    Code 80 nullability mismatch on date-filtered reads), then refresh MVs.
    if staging or silver:
        reapply_migrations(ch_seeder.cfg)
    refresh_intermediates(ch_seeder.cfg)

    # 5. Run each case's batch request and evaluate its expect rules.
    for case in test_yaml.cases:
        status, payload = analytics_api.call_request(case["request"])
        if status != 200:
            LOG.warning("HTTP %d; body: %r", status, payload)
        evaluate_case(case, payload, status)
