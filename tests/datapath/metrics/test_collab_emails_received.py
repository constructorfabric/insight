"""Emails received per person over a window, with the department as the peer pool.

Bronze: daily M365 email-activity report per person (send/receive/read counts).
Silver: deduped to one row per person/day with the received-email count. Gold serves
the requested member's per-person sum of emails received over the window; the peer
view is the distribution of those per-person sums across the member's department.
A re-synced duplicate row changes nothing.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_emails_received"

ERIN = "erin@example.com"


def test_collab_emails_received(spec: SpecRun) -> None:
    """Dec 01 .. Dec 31 takes the Dec 25 rows; erin's sum is 50 and the department of
    five spreads {10,20,30,40,50}."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "collab.emails_received",
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

    r.row("collab.emails_received", "period", entity_id=ERIN).equals(value=50)
    r.row("collab.emails_received", "peer", entity_id=ERIN).equals(
        target_value=50, p25=20, median=30, p75=40, min=10, max=50, n=5
    )
    received = one(r.series("collab.emails_received"), entity_id=ERIN)["points"]
    assert float(one(received, bucket_start="2026-12-25")["value"]) == 50.0
    by_tool = one(
        r.breakdown("collab.emails_received"),
        entity_id=ERIN,
        dimensions={"key": "tool", "value": "m365"},
    )
    assert float(by_tool["value"]) == 50.0


def test_collab_emails_received_empty_window(spec: SpecRun) -> None:
    """A window with no receive rows serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [
                    {"metric_key": "collab.emails_received", "views": [{"view": "period"}]}
                ],
            },
        }
    )
    assert r.status == 200
    r.row("collab.emails_received", "period", entity_id=ERIN).equals(value=None)
