"""Meeting-free days per person over a window, with the department as the peer pool.

Bronze: daily Teams meeting reports and Zoom participant sessions per person.
Silver: both sources deduped to per-person/day meeting durations. Gold counts a day
as meeting-free when the person has deliberate collaboration activity but no meeting
time; each member m in {alice:1 .. erin:5} is seeded m one-chat-message, zero-duration
Teams days, so the department spreads {1,2,3,4,5} and erin's count is 5.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one, some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_meeting_free_days"

ERIN = "erin@example.com"

ERIN_MEETING_FREE_DAYS = ("2026-12-23", "2026-12-24", "2026-12-25", "2026-12-26", "2026-12-27")


def test_collab_meeting_free_days(spec: SpecRun) -> None:
    """December takes erin's five zero-duration Teams days; the department spreads
    {1,2,3,4,5} and each of her days is a single-count bucket."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "collab.meeting_free_days",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("collab.meeting_free_days", "period", entity_id=ERIN).equals(value=5)
    r.row("collab.meeting_free_days", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    points = one(r.series("collab.meeting_free_days"), entity_id=ERIN)["points"]
    assert len(some(points, value=1.0)) == 5
    for day in ERIN_MEETING_FREE_DAYS:
        assert float(one(points, bucket_start=day)["value"]) == 1.0, (
            f"should be meeting-free: {day}"
        )


def test_collab_meeting_free_days_empty_window(spec: SpecRun) -> None:
    """A window with no Teams rows serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [
                    {"metric_key": "collab.meeting_free_days", "views": [{"view": "period"}]}
                ],
            },
        }
    )
    assert r.status == 200
    r.row("collab.meeting_free_days", "period", entity_id=ERIN).equals(value=None)
