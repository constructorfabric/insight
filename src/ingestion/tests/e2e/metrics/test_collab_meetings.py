"""Teams meeting metrics served per person over a window, with the department as peers.

Bronze: the daily Teams activity report per person (meetings attended, organized,
ad hoc, scheduled, audio hours). Silver: one deduplicated row per person/day. Gold
serves each metric as the person's sum over the window, Teams-only, and the peer view
as the distribution of those sums across the department; the same request serves the
focus-time share. A re-synced duplicate row changes nothing.
"""

from __future__ import annotations

import pytest
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_meetings"

ERIN = "erin@example.com"


def test_unified_collaboration_meeting_metrics(spec: SpecRun) -> None:
    """December over the department: erin's five Teams meeting metrics each serve 5 with
    the department spread {1,2,3,4,5}; her focus-time share is 37.5."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "collab.meeting_hours",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool"]},
                        ],
                    },
                    {
                        "metric_key": "collab.meetings_count",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool"]},
                        ],
                    },
                    {
                        "metric_key": "collab.meetings_organized",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool"]},
                        ],
                    },
                    {
                        "metric_key": "collab.adhoc_meetings",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool"]},
                        ],
                    },
                    {
                        "metric_key": "collab.scheduled_meetings",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool"]},
                        ],
                    },
                    {
                        "metric_key": "collab.focus_time_pct",
                        "views": [{"view": "period"}, {"view": "peer"}, {"view": "timeseries", "bucket": "day"}],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("collab.meeting_hours", "period", entity_id=ERIN).equals(value=5)
    r.row("collab.meeting_hours", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    r.row("collab.meeting_hours", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-12-25", "value": 5}
    )
    r.row("collab.meeting_hours", "breakdown", entity_id=ERIN, dimensions={"key": "tool", "value": "m365"}).equals(
        value=5
    )

    r.row("collab.meetings_count", "period", entity_id=ERIN).equals(value=5)
    r.row("collab.meetings_count", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    r.row("collab.meetings_count", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-12-25", "value": 5}
    )
    r.row("collab.meetings_count", "breakdown", entity_id=ERIN, dimensions={"key": "tool", "value": "m365"}).equals(
        value=5
    )

    r.row("collab.meetings_organized", "period", entity_id=ERIN).equals(value=5)
    r.row("collab.meetings_organized", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    r.row("collab.meetings_organized", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-12-25", "value": 5}
    )
    r.row("collab.meetings_organized", "breakdown", entity_id=ERIN, dimensions={"key": "tool", "value": "m365"}).equals(
        value=5
    )

    r.row("collab.adhoc_meetings", "period", entity_id=ERIN).equals(value=5)
    r.row("collab.adhoc_meetings", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    r.row("collab.adhoc_meetings", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-12-25", "value": 5}
    )
    r.row("collab.adhoc_meetings", "breakdown", entity_id=ERIN, dimensions={"key": "tool", "value": "m365"}).equals(
        value=5
    )

    r.row("collab.scheduled_meetings", "period", entity_id=ERIN).equals(value=5)
    r.row("collab.scheduled_meetings", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    r.row("collab.scheduled_meetings", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-12-25", "value": 5}
    )
    r.row("collab.scheduled_meetings", "breakdown", entity_id=ERIN, dimensions={"key": "tool", "value": "m365"}).equals(
        value=5
    )

    r.row("collab.focus_time_pct", "period", entity_id=ERIN).equals(value=37.5)
    r.row("collab.focus_time_pct", "peer", entity_id=ERIN).equals(
        target_value=37.5, p25=50, median=62.5, p75=75, min=37.5, max=87.5, n=5
    )
    r.row("collab.focus_time_pct", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-12-25", "value": 37.5}
    )


def test_unified_collaboration_meeting_metrics_empty_window(spec: SpecRun) -> None:
    """A window with no meeting rows serves an honest null for every metric."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [
                    {"metric_key": "collab.meeting_hours", "views": [{"view": "period"}]},
                    {"metric_key": "collab.meetings_count", "views": [{"view": "period"}]},
                    {"metric_key": "collab.meetings_organized", "views": [{"view": "period"}]},
                    {"metric_key": "collab.adhoc_meetings", "views": [{"view": "period"}]},
                    {"metric_key": "collab.scheduled_meetings", "views": [{"view": "period"}]},
                    {"metric_key": "collab.focus_time_pct", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("collab.meeting_hours", "period", entity_id=ERIN).equals(value=None)
    r.row("collab.meetings_count", "period", entity_id=ERIN).equals(value=None)
    r.row("collab.meetings_organized", "period", entity_id=ERIN).equals(value=None)
    r.row("collab.adhoc_meetings", "period", entity_id=ERIN).equals(value=None)
    r.row("collab.scheduled_meetings", "period", entity_id=ERIN).equals(value=None)
    r.row("collab.focus_time_pct", "period", entity_id=ERIN).equals(value=None)
