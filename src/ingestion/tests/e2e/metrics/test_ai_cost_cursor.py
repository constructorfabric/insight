"""AI usage cost fed by Cursor, served per person over a window in dollars.

Bronze: Cursor prices usage per event and ships each event twice, an hourly stream
and a next-day re-fetch carrying the finalized amount. Silver folds both into one
row per event (latest wins) and sums the day per developer. Gold sums those daily
amounts over the window: the charged amount already includes the token fee, usage
covered by seats still counts, and an active day with no priced event reads null.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_cost_cursor"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"


def test_ai_usage_cost_from_cursor_events(spec: SpecRun) -> None:
    """500 (fee inside, not added) + 300 (subscription-covered) + 250 (finalized, not
    doubled by the re-sync) = 1050 cents, served as 10.5 across every view."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [
                    {
                        "metric_key": "ai.cost",
                        "views": [
                            {"view": "period"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ai.cost", "period", entity_id=ALICE).equals(value=10.5)
    points = one(r.series("ai.cost"), entity_id=ALICE)["points"]
    assert float(one(points, bucket_start="2026-01-05")["value"]) == 10.5
    by_tool = one(r.breakdown("ai.cost"), entity_id=ALICE, dimensions={"key": "tool", "value": "cursor"})
    assert float(by_tool["value"]) == 10.5


def test_ai_usage_cost_rounds_the_daily_sum_once(spec: SpecRun) -> None:
    """Three 0.6-cent events sum to 1.8 cents and round once to 2 cents."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [{"metric_key": "ai.cost", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("ai.cost", "period", entity_id=CAROL).equals(value=0.02)


def test_ai_usage_cost_across_tools(spec: SpecRun) -> None:
    """Cursor and Claude Code on the same day sum to 2.03 and split by tool."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [BOB]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [
                    {
                        "metric_key": "ai.cost",
                        "views": [{"view": "period"}, {"view": "breakdown", "dimensions": ["tool"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ai.cost", "period", entity_id=BOB).equals(value=2.03)
    cursor = one(r.breakdown("ai.cost"), entity_id=BOB, dimensions={"key": "tool", "value": "cursor"})
    assert float(cursor["value"]) == 2.0
    claude_code = one(r.breakdown("ai.cost"), entity_id=BOB, dimensions={"key": "tool", "value": "claude_code"})
    assert float(claude_code["value"]) == 0.03


def test_ai_usage_cost_on_an_active_day_without_priced_events(spec: SpecRun) -> None:
    """An active Cursor day with no priced event is not tracked, not zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-01-06", "to": "2026-01-06"},
                "metrics": [{"metric_key": "ai.cost", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("ai.cost", "period", entity_id=ALICE).equals(value=None)
