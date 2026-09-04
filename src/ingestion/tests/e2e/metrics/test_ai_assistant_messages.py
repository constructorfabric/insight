"""AI assistant messages, served per person over a window.

Bronze: ChatGPT daily per-user chat activity (message counts). Silver: one row per
user/day carrying the chat-message count, deduped. Gold serves the sum of ChatGPT chat
messages over the window; the peer view is the department (alice 40, bob 20, carol 10).
Alice's row is re-synced once and must not double.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one, some
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_assistant_messages"

ALICE = "alice@example.com"


def test_ai_assistant_messages(spec: SpecRun) -> None:
    """January takes alice's single (re-synced) day: 40 messages, one chatgpt/chat breakdown row."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [
                    {
                        "metric_key": "ai.assistant_messages",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool", "surface"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ai.assistant_messages", "period", entity_id=ALICE).equals(value=40)
    r.row("ai.assistant_messages", "peer", entity_id=ALICE).equals(
        target_value=40, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    points = one(r.series("ai.assistant_messages"), entity_id=ALICE)["points"]
    assert float(one(points, bucket_start="2026-01-05")["value"]) == 40.0

    by_tool = one(r.breakdown("ai.assistant_messages"), entity_id=ALICE, dimensions={"key": "tool", "value": "chatgpt"})
    assert len(by_tool["dimensions"]) == 2
    assert some(by_tool["dimensions"], key="surface", value="chat")
    assert float(by_tool["value"]) == 40.0
