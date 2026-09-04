"""Unified collaboration messaging metrics across period, peer, timeseries and tool
breakdown views.

Bronze: M365 Teams per-user activity for five people on one day, private chat message
counts 10..50 and no team chat, with one row synced twice. Gold serves the sent count,
a 100% DM ratio and messages per active day per person; channel posts, with no team
chat in the data, serve as null with an empty tool breakdown. The duplicate row
changes nothing.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import MetricResponse, one
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_messages"

ERIN = "erin@example.com"

MESSAGING_VIEWS = [
    {"view": "period"},
    {"view": "peer"},
    {"view": "timeseries", "bucket": "day"},
    {"view": "breakdown", "dimensions": ["tool"]},
]


def erin_value_on(r: MetricResponse, key: str, bucket_start: str) -> float:
    points = one(r.series(key), entity_id=ERIN)["points"]
    return float(one(points, bucket_start=bucket_start)["value"])


def erin_value_for_tool(r: MetricResponse, key: str, tool: str) -> float:
    return float(one(r.breakdown(key), entity_id=ERIN, dimensions={"key": "tool", "value": tool})["value"])


def test_unified_collaboration_messaging(spec: SpecRun) -> None:
    """Erin's 50 private chat messages on Dec 25 are the sent count, a 100% DM ratio and
    50 per active day in every view; with no team chat, channel posts serve as null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {"metric_key": "collab.messages_sent", "views": MESSAGING_VIEWS},
                    {"metric_key": "collab.channel_posts", "views": MESSAGING_VIEWS},
                    {"metric_key": "collab.dm_ratio", "views": MESSAGING_VIEWS},
                    {"metric_key": "collab.msgs_per_active_day", "views": MESSAGING_VIEWS},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("collab.messages_sent", "period", entity_id=ERIN).equals(value=50)
    r.row("collab.messages_sent", "peer", entity_id=ERIN).equals(
        target_value=50, p25=20, median=30, p75=40, min=10, max=50, n=5
    )
    assert erin_value_on(r, "collab.messages_sent", "2026-12-25") == 50.0
    assert erin_value_for_tool(r, "collab.messages_sent", "m365") == 50.0

    r.row("collab.channel_posts", "period", entity_id=ERIN).equals(value=None)
    r.row("collab.channel_posts", "peer", entity_id=ERIN).equals(
        target_value=None, p25=None, median=None, p75=None, min=None, max=None, n=0
    )
    assert erin_value_on(r, "collab.channel_posts", "2026-12-25") == 0.0
    assert len(r.breakdown("collab.channel_posts")) == 0

    r.row("collab.dm_ratio", "period", entity_id=ERIN).equals(value=100)
    r.row("collab.dm_ratio", "peer", entity_id=ERIN).equals(
        target_value=100, p25=100, median=100, p75=100, min=100, max=100, n=5
    )
    assert erin_value_on(r, "collab.dm_ratio", "2026-12-25") == 100.0
    assert erin_value_for_tool(r, "collab.dm_ratio", "m365") == 100.0

    r.row("collab.msgs_per_active_day", "period", entity_id=ERIN).equals(value=50)
    r.row("collab.msgs_per_active_day", "peer", entity_id=ERIN).equals(
        target_value=50, p25=20, median=30, p75=40, min=10, max=50, n=5
    )
    assert erin_value_on(r, "collab.msgs_per_active_day", "2026-12-25") == 50.0
    assert erin_value_for_tool(r, "collab.msgs_per_active_day", "m365") == 50.0
