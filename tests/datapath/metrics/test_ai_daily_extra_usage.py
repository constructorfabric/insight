"""AI daily approximate extra usage cost: billed seat spend placed on the days it was spent.

Bronze: one per-seat spend snapshot per day the vendor was read, used_credits the
month-to-date total already in cents. Silver: `class_ai_overage_daily`, one reading per
seat per day. Gold serves a day as the step between consecutive readings inside one billing
month, after the running total is corrected to its suffix minimum, so a downward revision
rewrites the days already reported and never emits a negative day; a drop on the month's
last calendar day may raise the month but never lower it. Five Engineering seats give the
peer view its quartiles, and every window sum equals what ai.extra_usage_cost reports.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_daily_extra_usage"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"
DAVE = "dave@example.com"
ERIN = "erin@example.com"


def test_ai_daily_extra_usage_across_the_served_views(spec: SpecRun) -> None:
    """alice's steps are 100 / 200 / 0 cents, $1 / $2 / $0 with a $3 window sum; the peer
    pool is the five per-person sums {3, 4, 5, 7, 9}."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [
                    {
                        "metric_key": "ai.daily_approximate_extra_usage_cost",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries"},
                            {"view": "breakdown", "dimensions": ["tool"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ai.daily_approximate_extra_usage_cost", "period", entity_id=ALICE).equals(value=3.0)
    r.row("ai.daily_approximate_extra_usage_cost", "peer", entity_id=ALICE).equals(
        target_value=3.0, p25=4.0, median=5.0, p75=7.0, min=3.0, max=9.0, n=5
    )
    daily = one(r.series("ai.daily_approximate_extra_usage_cost"), entity_id=ALICE)["points"]
    assert float(one(daily, bucket_start="2026-12-05")["value"]) == 1.0
    assert float(one(daily, bucket_start="2026-12-06")["value"]) == 2.0
    assert float(one(daily, bucket_start="2026-12-07")["value"]) == 0.0
    by_tool = one(
        r.breakdown("ai.daily_approximate_extra_usage_cost"),
        entity_id=ALICE,
        dimensions={"key": "tool", "value": "claude"},
    )
    assert float(by_tool["value"]) == 3.0


def test_ai_daily_extra_usage_never_reports_a_negative_day(spec: SpecRun) -> None:
    """erin is read 500 then 400: the 400 is the month's truth, so Dec 05 is rewritten
    from 5.00 to 4.00 and Dec 06 is 0.00 where a naive difference would emit -1.00."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [
                    {
                        "metric_key": "ai.daily_approximate_extra_usage_cost",
                        "views": [{"view": "period"}, {"view": "timeseries"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ai.daily_approximate_extra_usage_cost", "period", entity_id=ERIN).equals(value=4.0)
    series = r.series("ai.daily_approximate_extra_usage_cost")
    daily = one(series, entity_id=ERIN)["points"]
    assert float(one(daily, bucket_start="2026-12-05")["value"]) == 4.0
    assert float(one(daily, bucket_start="2026-12-06")["value"]) == 0.0
    for entry in series:
        spent = [float(point["value"]) for point in entry["points"] if point["value"] is not None]
        assert all(day >= 0.0 for day in spent), (
            f"should never report a negative day: {entry['entity_id']} {spent!r}"
        )


def test_ai_daily_extra_usage_holds_a_month_against_its_last_day_reading(spec: SpecRun) -> None:
    """Held, Sep 29 keeps $6 and Sep 30 is $0; read as a correction the month would be $1.
    Both metrics are asked, because a hold on one side only reads as $6 against $1."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-09-01", "to": "2026-09-30"},
                "metrics": [
                    {
                        "metric_key": "ai.daily_approximate_extra_usage_cost",
                        "views": [{"view": "period"}, {"view": "timeseries"}],
                    },
                    {"metric_key": "ai.extra_usage_cost", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ai.daily_approximate_extra_usage_cost", "period", entity_id=ALICE).equals(value=6.0)
    r.row("ai.extra_usage_cost", "period", entity_id=ALICE).equals(value=6.0)
    daily = one(r.series("ai.daily_approximate_extra_usage_cost"), entity_id=ALICE)["points"]
    assert float(one(daily, bucket_start="2026-09-29")["value"]) == 6.0
    assert float(one(daily, bucket_start="2026-09-30")["value"]) == 0.0


def test_ai_daily_extra_usage_keeps_an_overstated_first_reading_erased(spec: SpecRun) -> None:
    """Oct 01 reads 5000 where the month spends 300: the rewriting rule makes it $1, and the
    last-day hold must not undo that by flooring on the largest earlier reading."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [BOB]},
                "period": {"from": "2026-10-01", "to": "2026-10-31"},
                "metrics": [
                    {
                        "metric_key": "ai.daily_approximate_extra_usage_cost",
                        "views": [{"view": "period"}, {"view": "timeseries"}],
                    },
                    {"metric_key": "ai.extra_usage_cost", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ai.daily_approximate_extra_usage_cost", "period", entity_id=BOB).equals(value=3.0)
    r.row("ai.extra_usage_cost", "period", entity_id=BOB).equals(value=3.0)
    daily = one(r.series("ai.daily_approximate_extra_usage_cost"), entity_id=BOB)["points"]
    assert float(one(daily, bucket_start="2026-10-01")["value"]) == 1.0
    assert float(one(daily, bucket_start="2026-10-31")["value"]) == 2.0


def test_ai_daily_extra_usage_sums_to_the_monthly_figure(spec: SpecRun) -> None:
    """Served side by side, the two metrics agree over a whole month for every seat,
    erin's corrected month included; they are alternatives, never addends."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE, BOB, CAROL, DAVE, ERIN]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [
                    {
                        "metric_key": "ai.daily_approximate_extra_usage_cost",
                        "views": [{"view": "period"}],
                    },
                    {"metric_key": "ai.extra_usage_cost", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    daily = r.rows("ai.daily_approximate_extra_usage_cost", "period")
    assert float(one(daily, entity_id=ALICE)["value"]) == 3.0
    assert float(one(daily, entity_id=BOB)["value"]) == 5.0
    assert float(one(daily, entity_id=CAROL)["value"]) == 7.0
    assert float(one(daily, entity_id=DAVE)["value"]) == 9.0
    assert float(one(daily, entity_id=ERIN)["value"]) == 4.0

    monthly = r.rows("ai.extra_usage_cost", "period")
    assert float(one(monthly, entity_id=ALICE)["value"]) == 3.0
    assert float(one(monthly, entity_id=BOB)["value"]) == 5.0
    assert float(one(monthly, entity_id=CAROL)["value"]) == 7.0
    assert float(one(monthly, entity_id=DAVE)["value"]) == 9.0
    assert float(one(monthly, entity_id=ERIN)["value"]) == 4.0


def test_ai_daily_extra_usage_empty_window(spec: SpecRun) -> None:
    """A window with no reading taken in range serves a null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-06-01", "to": "2026-06-30"},
                "metrics": [
                    {
                        "metric_key": "ai.daily_approximate_extra_usage_cost",
                        "views": [{"view": "period"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200
    r.row("ai.daily_approximate_extra_usage_cost", "period", entity_id=ALICE).equals(value=None)
