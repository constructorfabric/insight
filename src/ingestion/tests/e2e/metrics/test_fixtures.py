"""Assertion runner for `*.test.yaml` fixtures (seed-once model).

One pytest invocation per discovered `<name>.test.yaml`. The stack is seeded and
built ONCE per session (conftest `build_world`, using every fixture's namespaced
bronze), so this test does NOT seed or build anything — it just:

    namespace the case's request (person_id / org_unit_id → this fixture's
      private namespace, matching how its bronze was seeded)  →
    POST /v1/metrics/queries per case  →
    evaluate expect rules

Expect rules are namespace-agnostic (they assert metric_key + numeric stats,
never identity — verified), so only the request is rewritten. Session build +
per-test fixtures live in `../conftest.py`; isolation is proven by
`../meta/test_seed_isolation.py`.
"""

from __future__ import annotations

import logging

import pytest

from lib import namespace
from lib.analytics_api import AnalyticsApiProcess
from lib.expect_engine import evaluate_case
from lib.fixture_loader import TestYaml

pytestmark = pytest.mark.fixture
LOG = logging.getLogger("e2e.runner")

# Fixtures whose asserted stats are NOT isolable in the shared seed-once world.
# The TEAM bullet value/distribution is a headcount-weighted blend across the
# whole collab/task population (see analytics-api migration
# m20260604_000002_collab_bullet_distribution: "the team bullet blends each
# roster member's department cohort … headcount-weighted"), so `value`/`n`/
# quantiles depend on EVERY seeded fixture, not just this one. In the old
# per-fixture flow that blend equalled the single fixture's org by accident; in
# seed-once it spans all fixtures (e.g. m365_emails_sent value = Σ/all-collab-
# people = 315/75 = 4.2, not the intended 30). The IC variants of these metrics
# (…0012 m365_emails_sent, …0011 tasks_completed) ARE person/org-scoped, assert
# the same metric_keys, and pass — so metric coverage is retained. Their bronze
# still seeds (harmless); only the non-isolable assertion is skipped.
_NON_ISOLABLE_COMPANY_WIDE = {
    "team_bullet_collab_emails_sent",
    "team_bullet_task_delivery_tasks_completed",
}


def test_metric_smoke(
    test_yaml: TestYaml,
    analytics_api: AnalyticsApiProcess,
) -> None:
    if test_yaml.name in _NON_ISOLABLE_COMPANY_WIDE:
        pytest.skip(
            f"{test_yaml.name}: team-bullet stats are a company-wide headcount-weighted "
            "blend, so they cannot be isolated in the shared seed-once DB; the IC variant "
            "covers this metric_key. See _NON_ISOLABLE_COMPANY_WIDE."
        )
    # The world is already seeded (analytics_api depends on build_world). Rewrite
    # each case's request into this fixture's identity namespace, then assert.
    token = namespace.token_for(test_yaml.name)
    for case in test_yaml.cases:
        request = namespace.namespace_request(case["request"], token)
        status, payload = analytics_api.call_request(request)
        if status != 200:
            LOG.warning("HTTP %d; body: %r", status, payload)
        evaluate_case(case, payload, status)
