"""Slack guest accounts never reach chat metrics.

The Slack chat feeder drops an account proven to be a guest and keeps one whose
guest flag is unknown. The activity-derived flags (active, billable) never filter:
a member is kept even when both are false.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_chat_slack_guest_excluded"

MEMBER_MESSAGES = {"alice@example.com": 3, "bob@example.com": 5, "judy@example.com": 55}
GUESTS = ["frank@example.com", "ivan@example.com"]


def test_slack_guests_are_excluded_and_members_count(spec: SpecRun) -> None:
    """Members serve their message count whatever their activity flags; guests serve null."""
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
