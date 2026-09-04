"""Task reopen rate, served per person over a window, with the department peer view.

Bronze: Jira closed issues, their status-change history (close/reopen chains) and
users; BambooHR employees give the Engineering cohort. Silver derives one close or
reopen event per transition. Gold serves 100 * reopens / closes per person, gated
to at least 5 closes. A member with closes but no reopens rates NULL and leaves
the pool; a pool of four is below the peer minimum, so every percentile is withheld.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one, some
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_reopen_rate"

ERIN = "erin@example.com"


def test_tasks_reopen_rate(spec: SpecRun) -> None:
    """Erin's five-close chain rates 80; the four-person pool reports n but no percentiles,
    and per day a reopened close rates 100 while a day with no close carries no rate."""
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
                        "views": [{"view": "period"}, {"view": "peer"}, {"view": "timeseries", "bucket": "day"}],
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
    points = one(r.series("tasks.reopen_rate"), entity_id=ERIN)["points"]
    assert some(points, value=100.0)
    assert [point for point in points if point["value"] is None]


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
