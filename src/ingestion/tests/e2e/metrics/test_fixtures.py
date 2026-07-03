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

from lib.analytics import AnalyticsProcess
from lib.ch_seeder import CHSeeder
from lib.dbt_runner import DbtRunner
from lib.enrich import EnrichRunner
from lib.expect_engine import evaluate_case
from lib.fixture_loader import TestYaml
from lib.migration_applier import refresh_intermediates, reapply_migrations
from lib.worker import WorkerContext

pytestmark = pytest.mark.fixture
LOG = logging.getLogger("e2e.runner")


def test_metric_smoke(
    test_yaml: TestYaml,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    enrich_runner: EnrichRunner,
    analytics: AnalyticsProcess,
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
    # 3b. Connector enrich steps (descriptor.images.enrich), between staging and
    #     silver — mirrors prod: dbt(tag:<c>) → <c>-enrich → dbt(silver). Data-driven
    #     from descriptors, so any connector with an enrich step participates (jira
    #     today, youtrack once it ships one). The enrich binary reads the connector's
    #     staging tables (built above) and writes back into `staging.*`.
    touched_schemas = {schema for schema, _ in test_yaml.touched_tables}
    ran_enrich_steps = []
    for step in enrich_runner.steps_for(touched_schemas):
        source_ids = enrich_runner.discover_source_ids(step, test_yaml.touched_tables)
        if not source_ids:
            continue
        # The enrich binary APPENDS into its staging output tables, and dbt never
        # rebuilds them (they are sources, not models), so a prior test's rows for
        # the same source_id would survive into this test's silver rebuild and
        # inflate absolute-count metrics. Clear them before enriching so each test
        # starts from a clean enrich output (the silver class table read from them
        # is already truncated via the ledger above).
        for schema, table in dbt_runner.enrich_output_tables(step.name):
            ch_seeder.truncate_table(schema, table)
        enrich_runner.run(step, source_ids)
        ran_enrich_steps.append(step)

    # 3c. Silver class models. Build exactly what the seeded data supports:
    #     derive_selectors gives the silver fed by seeded bronze (e.g. class_task_users,
    #     class_task_field_metadata); each enrich step additionally feeds silver via an
    #     EPHEMERAL staging view (e.g. class_task_field_history), which derive_selectors
    #     can't see. We build that precise set BY NAME rather than the connector's broad
    #     `tag:silver,tag:<c>+` so unseeded streams (class_task_sprints, the identity
    #     chain, …) are not dragged in and fail on absent bronze. Only steps that
    #     ACTUALLY ran (had a source_id) contribute their ephemeral targets — otherwise
    #     we'd build silver that depends on enrich output that was never produced.
    silver_set = set(silver)
    for step in ran_enrich_steps:
        silver_set.update(dbt_runner.ephemeral_silver_targets(step.name))
    if silver_set:
        # Record before building (same rationale as staging above): a build that
        # raises partway still leaves the targets in the truncate ledger for the
        # next test to clean.
        for cls in silver_set:
            ch_seeder.ledger.record("silver", cls)
        dbt_runner.build(" ".join(sorted(silver_set)), worker_ctx=worker_ctx)

    # 4. Recreate gold views against the now-real silver schema (fixes the rig-only
    #    Code 80 nullability mismatch on date-filtered reads), then refresh MVs.
    if staging or silver_set or ran_enrich_steps:
        reapply_migrations(ch_seeder.cfg)
    refresh_intermediates(ch_seeder.cfg)

    # 5. Run each case's batch request and evaluate its expect rules.
    for case in test_yaml.cases:
        status, payload = analytics.call_request(case["request"])
        if status != 200:
            LOG.warning("HTTP %d; body: %r", status, payload)
        evaluate_case(case, payload, status)
