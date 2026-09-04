"""AI development conversations, served per person over a window.

Bronze: Codex daily per-user leaderboard (thread count as coding sessions). Silver:
deduped to one row per user/day carrying the session count. Gold serves the sum of
Codex coding sessions over the window; the peer view is the department distribution
(alice 12, bob 6, carol 3). A re-synced duplicate row must not double the count.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_dev_conversations"

ALICE = "alice@example.com"


def test_ai_development_conversations(spec: SpecRun) -> None:
    """January takes alice's single deduped day of 12 sessions; the department peer
    pool has n=3 and the codex breakdown carries the same 12."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [
                    {
                        "metric_key": "ai.dev_conversations",
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

    r.row("ai.dev_conversations", "period", entity_id=ALICE).equals(value=12)
    r.row("ai.dev_conversations", "peer", entity_id=ALICE).equals(
        target_value=12, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    points = one(r.series("ai.dev_conversations"), entity_id=ALICE)["points"]
    assert float(one(points, bucket_start="2026-01-05")["value"]) == 12.0
    by_tool = one(r.breakdown("ai.dev_conversations"), entity_id=ALICE, dimensions={"key": "tool", "value": "codex"})
    assert float(by_tool["value"]) == 12.0
