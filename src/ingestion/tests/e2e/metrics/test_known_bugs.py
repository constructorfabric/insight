"""xfail coverage for tracked product bugs — granular, so only the broken check is xfailed.

Each bug gets two tests over the SAME request, so the working behavior is asserted
normally and only the defect-affected check is marked xfail:

  * an "envelope" test (must PASS) — the request succeeds (HTTP 200 / result 'ok').
    This proves the bug is SILENT, not an error.
  * a "value" test (xfail, strict=True) — the numbers the defect corrupts. It FAILS
    today (documenting the bug; suite stays green as `xfailed`) and becomes a hard
    failure (`xpassed`) the moment the bug is fixed — the signal to delete the marker.

These use the seeded world (the `analytics` fixture), unlike the offline guards in meta/.

#1657 — the analytics service silently DROPS an unrecognized `org_unit_id eq` filter
(only `org_unit_id in` is implemented), so a department-scoped team-bullet query
returns HTTP 200 with the whole-COMPANY value. The `team_bullet_*.test.yaml` fixtures
work around this by scoping via `person_id IN (roster)`; here we assert the SAME
department stats through the natural `org_unit_id eq` filter the product should
support. In Python (not a `*.test.yaml`) so the request-scoping guard — which forbids
`org_unit_id eq` in fixtures — does not flag it.
"""

from __future__ import annotations

import copy
import re
from pathlib import Path

import pytest

from lib import namespace
from lib.analytics import AnalyticsProcess
from lib.expect_engine import evaluate_case
from lib.fixture_loader import load as load_test

_METRICS_ROOT = Path(__file__).resolve().parent

pytestmark = pytest.mark.smoke

_ORG_UNIT_EQ_FIXTURES = [
    "team_bullet_collab_emails_sent",
    "team_bullet_task_delivery_tasks_completed",
]

_XFAIL_1657 = pytest.mark.xfail(
    reason="#1657: analytics silently drops `org_unit_id eq` and returns the "
    "whole-company value; remove this marker when the API implements org_unit_id eq",
    strict=True,
)


def _org_unit_eq_case(fixture_name: str) -> tuple[dict, dict]:
    """Load a team-bullet fixture's 'department of 5' case and swap its
    `person_id IN (roster)` clause for the natural `org_unit_id eq '<dept>'` filter
    (#1657), preserving the metric_date window. Returns (namespaced case, request)."""
    ty = load_test(_METRICS_ROOT / f"{fixture_name}.test.yaml")
    case = copy.deepcopy(ty.cases[0])  # the "department of 5" case, not the empty-window one
    query = case["request"]["body"]["queries"][0]
    query["$filter"] = re.sub(
        r"person_id in \([^)]*\)", "org_unit_id eq 'Engineering'", query["$filter"]
    )
    assert "org_unit_id eq" in query["$filter"], "filter swap failed — fixture shape changed"
    request = namespace.namespace_request(case["request"], namespace.token_for(ty.name))
    return case, request


def _with_expect(case: dict, keep_equal: bool) -> dict:
    """A shallow clone of `case` keeping only the envelope expects (`keep_equal=False`:
    the pure status/result assertions) or only the value expects (`keep_equal=True`:
    the `find`+`equal` numeric assertion)."""
    return {**case, "expect": [e for e in case["expect"] if ("equal" in e) == keep_equal]}


@pytest.mark.parametrize("fixture_name", _ORG_UNIT_EQ_FIXTURES)
def test_org_unit_eq_query_succeeds_silently(analytics: AnalyticsProcess, fixture_name: str) -> None:
    """MUST PASS: an `org_unit_id eq` team-bullet query returns HTTP 200 / result 'ok'.
    #1657 is a SILENT defect — it does not error, it returns wrong numbers with a 200.
    Asserts only the envelope; the numbers are checked (and xfailed) below."""
    case, request = _org_unit_eq_case(fixture_name)
    status, payload = analytics.call_request(request)
    evaluate_case(_with_expect(case, keep_equal=False), payload, status)


@_XFAIL_1657
@pytest.mark.parametrize("fixture_name", _ORG_UNIT_EQ_FIXTURES)
def test_org_unit_eq_scopes_to_department(analytics: AnalyticsProcess, fixture_name: str) -> None:
    """xfail(strict): ONLY the company-affected numbers. Asserts the department stats
    a correct `org_unit_id eq` would return (value / median / p25 / p75 / range). Fails
    today because the dropped filter yields the company-wide blend (#1657); xpasses —
    a hard failure — when the API implements `org_unit_id eq`."""
    case, request = _org_unit_eq_case(fixture_name)
    status, payload = analytics.call_request(request)
    evaluate_case(_with_expect(case, keep_equal=True), payload, status)


# #1658 — the AI IC bullet (catalog id ...0013) and its team sibling (...0006) lost
# their p25/p75/n department distribution when m20260618_000001 (cc_overage)
# redefined both query_refs last, rebuilding from the pre-distribution shape and
# dropping the p25/p75/n that m20260611_000001 (dept-reconcile) had added. The
# bullets now return only value/median/range_min/range_max. The value/median/range
# checks still PASS (asserted by metrics/ai_cc_cost.test.yaml in the main suite);
# only the missing distribution is xfailed here. ai_cc_cost seeds an Engineering
# department whose per-person cc_cost sums are {60,120,180,240,300}c, so
# quantileExact gives p25=sorted[1]=120, p75=sorted[3]=240, n=5.
_XFAIL_1658 = pytest.mark.xfail(
    reason="#1658: m20260618 clobbered the AI IC/team bullet (...0013/...0006) p25/p75/n "
    "distribution added by m20260611; remove this marker when the query_ref re-emits them",
    strict=True,
)


@_XFAIL_1658
def test_ai_ic_bullet_emits_department_distribution(analytics: AnalyticsProcess) -> None:
    """xfail(strict): the AI IC bullet should surface the department p25/p75/n. The
    value/median/range checks already pass via ai_cc_cost.test.yaml; only these
    distribution stats are absent today (#1658). Asserts the values a fixed query_ref
    would return for ai_cc_cost's {60,120,180,240,300}c department distribution."""
    ty = load_test(_METRICS_ROOT / "ai_cc_cost.test.yaml")
    case = copy.deepcopy(ty.cases[0])  # the cc_cost "department of 5" custom-scope case
    numeric = [e for e in case["expect"] if "equal" in e]
    assert numeric, "ai_cc_cost fixture has no equal rule to build from"
    numeric[0]["equal"] = {"p25": 120, "p75": 240, "n": 5}  # the dropped distribution
    request = namespace.namespace_request(case["request"], namespace.token_for(ty.name))
    status, payload = analytics.call_request(request)
    evaluate_case({**case, "expect": numeric}, payload, status)
