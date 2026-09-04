"""AI accepted and removed lines, served per person over a window.

Bronze: Cursor daily per-developer usage report (accepted lines added and deleted).
Silver: one row per developer/day carrying the accepted-lines counts. Gold serves the
sum of AI-accepted and AI-removed lines over the window; the peer view is the
department distribution (alice 40, bob 20, carol 10). A re-synced duplicate of
alice's row changes nothing.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_lines"

ALICE = "alice@example.com"


def test_ai_accepted_and_removed_lines(spec: SpecRun) -> None:
    """January takes the single Jan 05 row per person; alice's accepted 40 and removed 8
    are served once despite the re-synced duplicate."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [
                    {
                        "metric_key": "ai.accepted_lines",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["tool"]},
                        ],
                    },
                    {
                        "metric_key": "ai.removed_lines",
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

    r.row("ai.accepted_lines", "period", entity_id=ALICE).equals(value=40)
    r.row("ai.accepted_lines", "peer", entity_id=ALICE).equals(
        target_value=40, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    accepted = one(r.series("ai.accepted_lines"), entity_id=ALICE)["points"]
    assert float(one(accepted, bucket_start="2026-01-05")["value"]) == 40.0
    by_tool = one(r.breakdown("ai.accepted_lines"), entity_id=ALICE, dimensions={"key": "tool", "value": "cursor"})
    assert float(by_tool["value"]) == 40.0

    r.row("ai.removed_lines", "period", entity_id=ALICE).equals(value=8)
    r.row("ai.removed_lines", "peer", entity_id=ALICE).equals(
        target_value=8, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    removed = one(r.series("ai.removed_lines"), entity_id=ALICE)["points"]
    assert float(one(removed, bucket_start="2026-01-05")["value"]) == 8.0
    by_tool = one(r.breakdown("ai.removed_lines"), entity_id=ALICE, dimensions={"key": "tool", "value": "cursor"})
    assert float(by_tool["value"]) == 8.0
