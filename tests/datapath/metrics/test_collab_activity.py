"""Collaboration active days and breadth, served per person over a window.

Bronze: daily M365 reports (email, Teams chat, OneDrive/SharePoint files). Silver:
one row per person/day carrying each signal's count. Gold counts the days on which
any deliberate signal (email sent, chat sent, file engaged or shared) is above zero;
passive email (received, read) does not count. Each member is seeded that many
single-email days, so the department spreads {1,2,3,4,5}; the peer view is that spread.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_activity"

ERIN = "erin@example.com"


def test_unified_collaboration_activity(spec: SpecRun) -> None:
    """Dec 23 .. Dec 29 holds every seeded day: erin has 5 active days over one tool,
    breadth is 1, and the department spreads {1,2,3,4,5}."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-23", "to": "2026-12-29"},
                "metrics": [
                    {
                        "metric_key": "collab.active_days",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool"]},
                        ],
                    },
                    {
                        "metric_key": "collab.breadth",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("collab.active_days", "period", entity_id=ERIN).equals(value=5)
    r.row("collab.active_days", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    active_days = one(r.series("collab.active_days"), entity_id=ERIN)["points"]
    assert float(one(active_days, bucket_start="2026-12-23")["value"]) == 1.0
    by_tool = one(
        r.breakdown("collab.active_days"),
        entity_id=ERIN,
        dimensions={"key": "tool", "value": "m365"},
    )
    assert float(by_tool["value"]) == 5.0

    r.row("collab.breadth", "period", entity_id=ERIN).equals(value=1)
    r.row("collab.breadth", "peer", entity_id=ERIN).equals(
        target_value=1, p25=1, median=1, p75=1, min=1, max=1, n=5
    )
    breadth = one(r.series("collab.breadth"), entity_id=ERIN)["points"]
    assert float(one(breadth, bucket_start="2026-12-23")["value"]) == 1.0


def test_unified_collaboration_activity_empty_window(spec: SpecRun) -> None:
    """A window with no seeded days serves null for both metrics, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [
                    {"metric_key": "collab.active_days", "views": [{"view": "period"}]},
                    {"metric_key": "collab.breadth", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("collab.active_days", "period", entity_id=ERIN).equals(value=None)
    r.row("collab.breadth", "period", entity_id=ERIN).equals(value=None)
