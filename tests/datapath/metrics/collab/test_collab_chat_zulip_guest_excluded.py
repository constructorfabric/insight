"""Zulip guest accounts never reach chat metrics.

The Zulip chat feeder drops a sender whose role is guest and keeps every other
role, including an unknown one.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_chat_zulip_guest_excluded"

MEMBER_MESSAGES = {"alice@example.com": 3, "bob@example.com": 5, "judy@example.com": 55}
GUESTS = ["frank@example.com"]


def test_zulip_guests_are_excluded_and_members_count(spec: SpecRun) -> None:
    """Members serve their message count whatever their role; guests serve null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [*MEMBER_MESSAGES, *GUESTS]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [{"metric_key": "collab.messages_sent", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200

    for email, messages in MEMBER_MESSAGES.items():
        r.row("collab.messages_sent", "period", entity_id=email).equals(value=messages)
    for email in GUESTS:
        r.row("collab.messages_sent", "period", entity_id=email).equals(value=None)
