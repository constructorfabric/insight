"""AI PRs with assistant and PRs total, served per person over a window.

Bronze: Claude Team per-user daily code metrics carrying the vendor's pull-request
attribution counters. Silver: one row per user/day, qualifying on the PR counters
alone. Gold serves each metric as the sum of its counter over the window; the peer
view is the department distribution. The seat_status breakdown is asserted here
because this is the only metric pair whose source carries the vendor status to gold.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_pr_attribution"

ALICE = "alice@example.com"


def test_ai_pull_request_attribution(spec: SpecRun) -> None:
    """January sums each counter once despite the re-synced duplicate: alice has 6 PRs
    with assistant out of 10, and the department of 3 spreads {6,3,1} and {10,8,4}."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [
                    {
                        "metric_key": "ai.prs_with_assistant",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["seat_status"]},
                        ],
                    },
                    {
                        "metric_key": "ai.prs_total",
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

    r.row("ai.prs_with_assistant", "period", entity_id=ALICE).equals(value=6)
    r.row("ai.prs_with_assistant", "peer", entity_id=ALICE).equals(
        target_value=6, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    with_assistant = one(r.series("ai.prs_with_assistant"), entity_id=ALICE)["points"]
    assert float(one(with_assistant, bucket_start="2026-01-05")["value"]) == 6.0
    by_seat_status = one(
        r.breakdown("ai.prs_with_assistant"), entity_id=ALICE, dimensions={"key": "seat_status", "value": "active"}
    )
    assert float(by_seat_status["value"]) == 6.0

    r.row("ai.prs_total", "period", entity_id=ALICE).equals(value=10)
    r.row("ai.prs_total", "peer", entity_id=ALICE).equals(
        target_value=10, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    total = one(r.series("ai.prs_total"), entity_id=ALICE)["points"]
    assert float(one(total, bucket_start="2026-01-05")["value"]) == 10.0
    by_tool = one(r.breakdown("ai.prs_total"), entity_id=ALICE, dimensions={"key": "tool", "value": "claude_code"})
    assert float(by_tool["value"]) == 10.0
