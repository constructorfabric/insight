"""Claude Team seat spend: extra usage cost and utilisation, served per person over a window.

Bronze: one per-seat spend snapshot, used_credits and monthly_credit_limit already in
cents. Silver: the latest snapshot per seat per calendar month. Gold serves one evidence
row per seat-month dated at the first day of the billing month, so a snapshot read Dec 10
lands in a window ending Dec 12 as December spend. used_credits is the money itself, not
the excess over the ceiling; a seat with no ceiling has cost but a null utilisation, and a
repeated sync of the same snapshot changes nothing.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_seat_extra_usage"

ERIN = "erin@example.com"
FRANK = "frank@example.com"


def test_ai_seat_extra_usage(spec: SpecRun) -> None:
    """Nov 01 .. Dec 12 takes the Dec 10 snapshot as December spend: erin's 250 cents is
    2.50 and 25% of her ceiling; the department spreads {0.5..2.5} and {5..25} over five."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [
                    {
                        "metric_key": "ai.extra_usage_cost",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["seat_tier"]},
                        ],
                    },
                    {
                        "metric_key": "ai.extra_usage_utilisation",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["seat_tier"]},
                        ],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ai.extra_usage_cost", "period", entity_id=ERIN).equals(value=2.5)
    r.row("ai.extra_usage_cost", "peer", entity_id=ERIN).equals(
        target_value=2.5, p25=1, median=1.5, p75=2, min=0.5, max=2.5, n=5
    )
    cost = one(r.series("ai.extra_usage_cost"), entity_id=ERIN)["points"]
    assert float(one(cost, bucket_start="2026-12-01")["value"]) == 2.5
    by_tier = one(
        r.breakdown("ai.extra_usage_cost"), entity_id=ERIN, dimensions={"key": "seat_tier", "value": "team_tier_1"}
    )
    assert float(by_tier["value"]) == 2.5

    r.row("ai.extra_usage_utilisation", "period", entity_id=ERIN).equals(value=25.0)
    r.row("ai.extra_usage_utilisation", "peer", entity_id=ERIN).equals(
        target_value=25.0, p25=10.0, median=15.0, p75=20.0, min=5.0, max=25.0, n=5
    )
    utilisation = one(r.series("ai.extra_usage_utilisation"), entity_id=ERIN)["points"]
    assert float(one(utilisation, bucket_start="2026-12-01")["value"]) == 25.0
    by_tier = one(
        r.breakdown("ai.extra_usage_utilisation"),
        entity_id=ERIN,
        dimensions={"key": "seat_tier", "value": "team_tier_1"},
    )
    assert float(by_tier["value"]) == 25.0


def test_ai_seat_with_no_ceiling(spec: SpecRun) -> None:
    """A seat with no ceiling: the money is served, the utilisation is an honest null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [FRANK]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [
                    {"metric_key": "ai.extra_usage_cost", "views": [{"view": "period"}]},
                    {"metric_key": "ai.extra_usage_utilisation", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("ai.extra_usage_cost", "period", entity_id=FRANK).equals(value=3.0)
    r.row("ai.extra_usage_utilisation", "period", entity_id=FRANK).equals(value=None)


def test_ai_seat_extra_usage_empty_window(spec: SpecRun) -> None:
    """A window with no snapshot read in range serves null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-06-01", "to": "2026-06-30"},
                "metrics": [{"metric_key": "ai.extra_usage_cost", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("ai.extra_usage_cost", "period", entity_id=ERIN).equals(value=None)
