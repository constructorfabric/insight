"""Slack accounts outside the billable membership never reach chat metrics.

The Slack chat feeder keeps only billable, non-guest accounts: a guest, a
deactivated account, a billable guest and an account with unknown flags serve
null. A billable member is kept whatever the per-day activity flag says, since
that flag reports activity on the day, not the account's status.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_chat_slack_nonmember_excluded"

MEMBER_MESSAGES = {"alice@example.com": 3, "bob@example.com": 5}
EXCLUDED = ["frank@example.com", "grace@example.com", "ivan@example.com", "judy@example.com"]


def test_only_billable_non_guest_slack_accounts_count(spec: SpecRun) -> None:
    """Billable members serve their message count; every other account serves null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [*MEMBER_MESSAGES, *EXCLUDED]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [{"metric_key": "collab.messages_sent", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200

    for email, messages in MEMBER_MESSAGES.items():
        r.row("collab.messages_sent", "period", entity_id=email).equals(value=messages)
    for email in EXCLUDED:
        r.row("collab.messages_sent", "period", entity_id=email).equals(value=None)
