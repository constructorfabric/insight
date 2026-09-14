"""Task reopen rate, served per person over a window, with the department peer view.

Bronze: Jira closed issues, their status-change history (close/reopen chains) and
users; BambooHR employees give the Engineering cohort. Silver derives one close or
reopen event per transition. Gold serves 100 * reopens / closes per person, gated
to at least 5 closes. A member with closes but no reopens rates NULL and leaves
the pool; a pool of four is below the peer minimum, so every percentile is withheld.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import approx, one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_reopen_rate"

ERIN = "erin@example.com"


def test_tasks_reopen_rate(spec: SpecRun) -> None:
    """Erin's five-close chain rates 80; the four-person pool reports n but no percentiles.
    Per day a reopened close rates 100, the close never undone rates null over the close it
    still counts, and a day with no close counts nothing at all."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.reopen_rate",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.reopen_rate", "period", entity_id=ERIN).equals(value=80)
    r.row("tasks.reopen_rate", "peer", entity_id=ERIN).equals(
        target_value=80, p25=None, median=None, p75=None, min=None, max=None, n=4
    )
    # ROE-1 closes on the odd days 03-21..03-29 and reopens on the even ones between,
    # so the last close is the one never undone and 03-30 holds no close at all.
    points = one(r.series("tasks.reopen_rate"), entity_id=ERIN)["points"]
    assert one(points, bucket_start="2026-03-21")["value"] == approx(100.0)

    undone = one(points, bucket_start="2026-03-29")
    assert undone["value"] is None, f"a rate over no reopens is null, not zero: {undone!r}"
    assert undone["denominator"] == approx(1.0), f"the close it counted is gone: {undone!r}"

    quiet = one(points, bucket_start="2026-03-30")
    assert quiet["value"] is None and "denominator" not in quiet, (
        f"a day with no close counts nothing: {quiet!r}"
    )


def test_tasks_reopen_rate_empty_window(spec: SpecRun) -> None:
    """A window with no closes serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "tasks.reopen_rate", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.reopen_rate", "period", entity_id=ERIN).equals(value=None)
