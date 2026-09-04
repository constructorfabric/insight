"""Claude Team seat cost per person, priced by the tier the seat carries.

Bronze: the vendor's invoice (its own money row plus one row per line) and the
per-seat spend snapshot naming each seat's tier. Silver keeps each line's per-seat
amount and billing month. Gold prices a seat only from a non-proration subscription
line, joined to the seat's tier through the operator-written tier map, dated at the
first day of the billing month; a seat with no tier, or a tier no priced line binds
to, is served as null rather than a share of the invoice total.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import approx, one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_seat_cost"

ALICE = "alice@example.com"
DAVE = "dave@example.com"
HEIDI = "heidi@example.com"
FRANK = "frank@example.com"


def test_ai_seat_cost_on_the_standard_tier(spec: SpecRun) -> None:
    """A Standard seat costs 12.00; the peer spread runs across both priced tiers,
    so the median pins the aggregation against a mean of 17.20."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [
                    {
                        "metric_key": "ai.seat_cost",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["seat_tier"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ai.seat_cost", "period", entity_id=ALICE).equals(value=12.0)
    r.row("ai.seat_cost", "peer", entity_id=ALICE).equals(
        target_value=12.0, p25=12.0, median=12.0, p75=25.0, min=12.0, max=25.0, n=5
    )
    points = one(r.series("ai.seat_cost"), entity_id=ALICE)["points"]
    assert float(one(points, bucket_start="2026-12-01")["value"]) == approx(12.0)
    by_tier = one(
        r.breakdown("ai.seat_cost"),
        entity_id=ALICE,
        dimensions={"key": "seat_tier", "value": "team_standard"},
    )
    assert float(by_tier["value"]) == approx(12.0)


def test_ai_seat_cost_on_the_premium_tier(spec: SpecRun) -> None:
    """The other priced tier costs the other amount."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [DAVE]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [{"metric_key": "ai.seat_cost", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("ai.seat_cost", "period", entity_id=DAVE).equals(value=25.0)


def test_ai_seat_cost_for_a_tier_no_priced_line_binds_to(spec: SpecRun) -> None:
    """A tier no priced line binds to is absent, not a share of the invoice total."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [HEIDI]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [{"metric_key": "ai.seat_cost", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("ai.seat_cost", "period", entity_id=HEIDI).equals(value=None)


def test_ai_seat_cost_for_a_seat_with_no_tier(spec: SpecRun) -> None:
    """A seat with no tier at all was never billed for."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [FRANK]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [{"metric_key": "ai.seat_cost", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("ai.seat_cost", "period", entity_id=FRANK).equals(value=None)


def test_ai_seat_cost_empty_window(spec: SpecRun) -> None:
    """The invoice month sits outside the window, so the seat cost is null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-06-01", "to": "2026-06-30"},
                "metrics": [{"metric_key": "ai.seat_cost", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("ai.seat_cost", "period", entity_id=ALICE).equals(value=None)
