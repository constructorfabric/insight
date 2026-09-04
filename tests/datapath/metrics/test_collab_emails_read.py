"""Emails read per person over a window, from the M365 email-activity report.

Bronze: the daily M365 email-activity report per person (send, receive and read
counts). Silver: one row per person/day with the read-email count, duplicates
collapsed. Gold serves the requested member's per-person sum of emails read over
the window; the peer view is the distribution of those sums across the member's
department. A re-synced duplicate row changes nothing.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_emails_read"

ERIN = "erin@example.com"


def test_collab_emails_read(spec: SpecRun) -> None:
    """December takes each member's one Dec 25 read-row; erin's sum is 50 and the
    department of 5 spreads {10,20,30,40,50}."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "collab.emails_read",
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

    r.row("collab.emails_read", "period", entity_id=ERIN).equals(value=50)
    r.row("collab.emails_read", "peer", entity_id=ERIN).equals(
        target_value=50, p25=20, median=30, p75=40, min=10, max=50, n=5
    )
    points = one(r.series("collab.emails_read"), entity_id=ERIN)["points"]
    assert float(one(points, bucket_start="2026-12-25")["value"]) == 50.0
    by_tool = one(
        r.breakdown("collab.emails_read"),
        entity_id=ERIN,
        dimensions={"key": "tool", "value": "m365"},
    )
    assert float(by_tool["value"]) == 50.0


def test_collab_emails_read_empty_window(spec: SpecRun) -> None:
    """A window with no read-rows serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [{"metric_key": "collab.emails_read", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("collab.emails_read", "period", entity_id=ERIN).equals(value=None)
