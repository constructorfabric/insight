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

No fixture is skip-listed: every metric — including the team bullets — is made
isolable by namespacing the identity dimension the query actually filters on.
Team bullets scope by `person_id IN (roster)` (how the API scopes team
aggregates), not `org_unit_id eq` (which the API does not implement and would
silently drop, blending the value company-wide).
"""

from __future__ import annotations

import logging

import pytest

from lib import namespace
from lib.analytics import AnalyticsProcess
from lib.expect_engine import evaluate_case
from lib.fixture_loader import TestYaml

pytestmark = pytest.mark.metric
LOG = logging.getLogger("e2e.runner")


def test_metric_smoke(
    test_yaml: TestYaml,
    analytics: AnalyticsProcess,
) -> None:
    # The world is already seeded (analytics depends on build_world). Rewrite
    # each case's request into this fixture's identity namespace, then assert.
    token = namespace.token_for(test_yaml.name)
    for case in test_yaml.cases:
        request = namespace.namespace_request(case["request"], token)
        status, payload = analytics.call_request(request)
        if status != 200:
            LOG.warning("HTTP %d; body: %r", status, payload)
        evaluate_case(case, payload, status)
