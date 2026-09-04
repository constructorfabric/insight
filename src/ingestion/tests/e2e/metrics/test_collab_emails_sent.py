"""Emails sent per person over a window, with the department as the peer pool.

Bronze: the daily M365 email-activity report per person (send, receive and read
counts). Silver: one row per person/day carrying the sent-email count, deduplicated.
Gold serves the requested member's per-person sum of emails sent over the window;
the peer view is the distribution of those sums across the member's department.
A re-synced duplicate row is deduplicated, not counted twice.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_emails_sent"

ERIN = "erin@example.com"


def test_collab_emails_sent(spec: SpecRun) -> None:
    """Erin's single Dec 25 send-row of 50 is her window sum; the department of five
    spreads {10,20,30,40,50}, so median 30, p25 20, p75 40, range [10,50]."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "collab.emails_sent",
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

    r.row("collab.emails_sent", "period", entity_id=ERIN).equals(value=50)
    r.row("collab.emails_sent", "peer", entity_id=ERIN).equals(
        target_value=50, p25=20, median=30, p75=40, min=10, max=50, n=5
    )
    sent = one(r.series("collab.emails_sent"), entity_id=ERIN)["points"]
    assert float(one(sent, bucket_start="2026-12-25")["value"]) == 50.0
    by_tool = one(r.breakdown("collab.emails_sent"), entity_id=ERIN, dimensions={"key": "tool", "value": "m365"})
    assert float(by_tool["value"]) == 50.0


def test_collab_emails_sent_empty_window(spec: SpecRun) -> None:
    """A window with no rows serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [{"metric_key": "collab.emails_sent", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("collab.emails_sent", "period", entity_id=ERIN).equals(value=None)
