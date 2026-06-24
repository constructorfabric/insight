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
from e2e_lib.enrich import EnrichRunner
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
    enrich_runner: EnrichRunner,
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
        dbt_runner.build(" ".join(f"+{m}" for m in staging), worker_ctx=worker_ctx)

    # 3b. Connector enrich steps (descriptor.images.enrich), between staging and
    #     silver — mirrors prod: dbt(tag:<c>) → <c>-enrich → dbt(silver). Data-driven
    #     from descriptors, so any connector with an enrich step participates (jira
    #     today, youtrack once it ships one). The enrich binary reads the connector's
    #     staging tables (built above) and writes back into `staging.*`.
    touched_schemas = {schema for schema, _ in test_yaml.touched_tables}
    enrich_steps = enrich_runner.steps_for(touched_schemas)
    enrich_ran = False
    for step in enrich_steps:
        source_ids = enrich_runner.discover_source_ids(step, test_yaml.touched_tables)
        if not source_ids:
            continue
        enrich_runner.run(step, source_ids)
        enrich_ran = True

    # 3c. Silver class models. Build exactly what the seeded data supports:
    #     derive_selectors gives the silver fed by seeded bronze (e.g. class_task_users,
    #     class_task_field_metadata); each enrich step additionally feeds silver via an
    #     EPHEMERAL staging view (e.g. class_task_field_history), which derive_selectors
    #     can't see. We build that precise set BY NAME rather than the connector's broad
    #     `tag:silver,tag:<c>+` so unseeded streams (class_task_sprints, the identity
    #     chain, …) are not dragged in and fail on absent bronze.
    silver_set = set(silver)
    for step in enrich_steps:
        silver_set.update(dbt_runner.ephemeral_silver_targets(step.name))
    if silver_set:
        dbt_runner.build(" ".join(sorted(silver_set)), worker_ctx=worker_ctx)
        for cls in silver_set:
            ch_seeder.ledger.record("silver", cls)

    # 4. Recreate gold views against the now-real silver schema (fixes the rig-only
    #    Code 80 nullability mismatch on date-filtered reads), then refresh MVs.
    if staging or silver_set or enrich_ran:
        reapply_migrations(ch_seeder.cfg)
    refresh_intermediates(ch_seeder.cfg)

    # 5. Run each case's batch request and evaluate its expect rules.
    for case in test_yaml.cases:
        status, payload = analytics_api.call_request(case["request"])
        if status != 200:
            LOG.warning("HTTP %d; body: %r", status, payload)
        evaluate_case(case, payload, status)
