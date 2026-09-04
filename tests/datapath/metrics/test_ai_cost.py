"""AI cost: Claude Code usage priced at the vendor's rates, served per person over a window.

Bronze: Claude Team daily per-user metrics carrying the cost as a decimal string.
Silver: one row per user/day with that amount in cents. Gold serves the window sum in
dollars per person; the peer view is the department distribution of the per-person
sums. The last case pairs it with ai.extra_usage_cost, which prices only what the
vendor billed on top of the seat fee: a subset served side by side, never added.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_cost"

ERIN = "erin@example.com"


def test_ai_cost(spec: SpecRun) -> None:
    """Nov 01 .. Dec 12 takes the Nov 15 and Dec 10 rows; per-person sums are 3r cents,
    so the department spreads {0.6,1.2,1.8,2.4,3} and erin's is 3."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [
                    {
                        "metric_key": "ai.cost",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ai.cost", "period", entity_id=ERIN).equals(value=3)
    r.row("ai.cost", "peer", entity_id=ERIN).equals(
        target_value=3, p25=1.2, median=1.8, p75=2.4, min=0.6, max=3, n=5
    )
    cost = one(r.series("ai.cost"), entity_id=ERIN)["points"]
    assert float(one(cost, bucket_start="2026-11-15")["value"]) == 2.0
    assert float(one(cost, bucket_start="2026-12-10")["value"]) == 1.0
    by_tool = one(
        r.breakdown("ai.cost"), entity_id=ERIN, dimensions={"key": "tool", "value": "claude_code"}
    )
    assert float(by_tool["value"]) == 3.0


def test_ai_cost_empty_window(spec: SpecRun) -> None:
    """A window with no rows serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-06-01", "to": "2026-06-30"},
                "metrics": [{"metric_key": "ai.cost", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("ai.cost", "period", entity_id=ERIN).equals(value=None)


def test_ai_cost_and_extra_usage_served_side_by_side(spec: SpecRun) -> None:
    """Each key returns its own value from its own source; extra usage stays a strict
    subset of priced consumption (0.75 <= 3.00), never blended into one figure."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [
                    {"metric_key": "ai.cost", "views": [{"view": "period"}]},
                    {"metric_key": "ai.extra_usage_cost", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("ai.cost", "period", entity_id=ERIN).equals(value=3)
    r.row("ai.extra_usage_cost", "period", entity_id=ERIN).equals(value=0.75)
