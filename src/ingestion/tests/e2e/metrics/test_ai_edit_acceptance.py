"""AI accepted edits and tool acceptance rate, served per person over a window.

Bronze: Claude Enterprise per-user/day usage (code tool suggestions accepted and
rejected). Silver: `class_ai_dev_usage`, one row per user/day. Gold serves the
accepted-edit count and the acceptance ratio per person over the window; the peer
view is the department distribution. A re-synced duplicate row changes nothing.
"""

from __future__ import annotations

import pytest
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_edit_acceptance"

ERIN = "erin@example.com"


def _point_values(series: list[dict], entity_id: str) -> dict[str, float]:
    """Bucket -> value for the buckets that hold one; an empty bucket is served as null."""
    entry = next(s for s in series if s["entity_id"] == entity_id)
    return {p["bucket_start"]: float(p["value"]) for p in entry["points"] if p["value"] is not None}


def _tool_value(breakdown: list[dict], entity_id: str, tool: str) -> float:
    for row in breakdown:
        if row["entity_id"] != entity_id:
            continue
        if any(d["key"] == "tool" and d["value"] == tool for d in row["dimensions"]):
            return float(row["value"])
    raise AssertionError(f"no breakdown row for {entity_id} tool={tool}")


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
    accepted_points = _point_values(r.series("ai.accepted_edit_actions"), ERIN)
    assert accepted_points["2026-11-15"] == 45.0
    assert accepted_points["2026-12-10"] == 35.0
    assert _tool_value(r.breakdown("ai.accepted_edit_actions"), ERIN, "claude_code") == 80.0

    r.row("ai.tool_acceptance_rate", "period", entity_id=ERIN).equals(value=40)
    r.row("ai.tool_acceptance_rate", "peer", entity_id=ERIN).equals(
        target_value=40, p25=25, median=30, p75=35, min=20, max=40, n=5
    )
    rate_points = _point_values(r.series("ai.tool_acceptance_rate"), ERIN)
    assert rate_points["2026-11-15"] == 45.0
    assert rate_points["2026-12-10"] == 35.0
    assert _tool_value(r.breakdown("ai.tool_acceptance_rate"), ERIN, "claude_code") == 40.0


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
