"""Unified assistant actions and chat conversations from Claude Enterprise.

Bronze: one Claude Enterprise per-user/day usage row per persona carrying chat
conversation, chat message and cowork action counts. Gold serves the assistant-action
count and the chat-conversation count per person over the window, the peer view is
the spread across the five personas, and the breakdown keys each count by tool and
surface.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one, some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_assistant_activity"

ERIN = "erin@example.com"


def test_ai_assistant_actions_and_chat_conversations(spec: SpecRun) -> None:
    """One Jan 05 row per persona: erin's 50 cowork actions and 25 chat conversations,
    each broken down by the two dimensions tool and surface."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [
                    {
                        "metric_key": "ai.assistant_actions",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool", "surface"]},
                        ],
                    },
                    {
                        "metric_key": "ai.chat_assistant_conversations",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool", "surface"]},
                        ],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ai.assistant_actions", "period", entity_id=ERIN).equals(value=50)
    r.row("ai.assistant_actions", "peer", entity_id=ERIN).equals(
        target_value=50, p25=20, median=30, p75=40, min=10, max=50, n=5
    )
    actions = one(r.series("ai.assistant_actions"), entity_id=ERIN)["points"]
    assert float(one(actions, bucket_start="2026-01-05")["value"]) == 50.0
    claude_actions = some(
        r.breakdown("ai.assistant_actions"),
        entity_id=ERIN,
        dimensions={"key": "tool", "value": "claude"},
    )
    cowork = one(claude_actions, dimensions={"key": "surface", "value": "cowork"})
    assert len(cowork["dimensions"]) == 2
    assert float(cowork["value"]) == 50.0

    r.row("ai.chat_assistant_conversations", "period", entity_id=ERIN).equals(value=25)
    r.row("ai.chat_assistant_conversations", "peer", entity_id=ERIN).equals(
        target_value=25, p25=10, median=15, p75=20, min=5, max=25, n=5
    )
    conversations = one(r.series("ai.chat_assistant_conversations"), entity_id=ERIN)["points"]
    assert float(one(conversations, bucket_start="2026-01-05")["value"]) == 25.0
    claude_conversations = some(
        r.breakdown("ai.chat_assistant_conversations"),
        entity_id=ERIN,
        dimensions={"key": "tool", "value": "claude"},
    )
    chat = one(claude_conversations, dimensions={"key": "surface", "value": "chat"})
    assert len(chat["dimensions"]) == 2
    assert float(chat["value"]) == 25.0
