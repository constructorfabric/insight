"""AI accepted edits and tool acceptance rate, served per person over a window.

Bronze: Claude Enterprise per-user/day usage (code tool suggestions accepted and
rejected). Silver: `class_ai_dev_usage`, one row per user/day. Gold serves the
accepted-edit count and the acceptance ratio per person over the window; the peer
view is the department distribution. A re-synced duplicate row changes nothing.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_edit_acceptance"

ERIN = "erin@example.com"


def test_accepted_edits_and_acceptance_rate_over_a_custom_window(spec: SpecRun) -> None:
    """Nov 01 .. Dec 12 takes the Nov 15 and Dec 10 rows; the per-person ratio is b+15,
    so the department spreads {20,25,30,35,40} and erin's is 40."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-11-01", "to": "2026-12-12"},
                "metrics": [
                    {
                        "metric_key": "ai.accepted_edit_actions",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool"]},
                        ],
                    },
                    {
                        "metric_key": "ai.tool_acceptance_rate",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool"]},
                        ],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ai.accepted_edit_actions", "period", entity_id=ERIN).equals(value=80)
    r.row("ai.accepted_edit_actions", "peer", entity_id=ERIN).equals(
        target_value=80, p25=50, median=60, p75=70, min=40, max=80, n=5
    )
    accepted = one(r.series("ai.accepted_edit_actions"), entity_id=ERIN)["points"]
    assert float(one(accepted, bucket_start="2026-11-15")["value"]) == 45.0
    assert float(one(accepted, bucket_start="2026-12-10")["value"]) == 35.0
    by_tool = one(
        r.breakdown("ai.accepted_edit_actions"), entity_id=ERIN, dimensions={"key": "tool", "value": "claude_code"}
    )
    assert float(by_tool["value"]) == 80.0

    r.row("ai.tool_acceptance_rate", "period", entity_id=ERIN).equals(value=40)
    r.row("ai.tool_acceptance_rate", "peer", entity_id=ERIN).equals(
        target_value=40, p25=25, median=30, p75=35, min=20, max=40, n=5
    )
    rate = one(r.series("ai.tool_acceptance_rate"), entity_id=ERIN)["points"]
    assert float(one(rate, bucket_start="2026-11-15")["value"]) == 45.0
    assert float(one(rate, bucket_start="2026-12-10")["value"]) == 35.0
    by_tool = one(
        r.breakdown("ai.tool_acceptance_rate"), entity_id=ERIN, dimensions={"key": "tool", "value": "claude_code"}
    )
    assert float(by_tool["value"]) == 40.0


def test_acceptance_rate_over_an_empty_window_is_null(spec: SpecRun) -> None:
    """A window with no rows serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-06-01", "to": "2026-06-30"},
                "metrics": [{"metric_key": "ai.tool_acceptance_rate", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("ai.tool_acceptance_rate", "period", entity_id=ERIN).equals(value=None)
