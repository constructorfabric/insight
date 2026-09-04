"""Stale in-progress task count, served per person as a point-in-time snapshot.

Bronze: Jira issues whose current status is In Progress, with no changelog rows, so
the synthetic initial status event sits at `created` and every open issue is older
than 14 days. Gold stamps the count on today's date rather than a close date, so a
window must include today to see it; the peer view is the department distribution
over five members holding 1..5 open issues each.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one, some
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_stale_in_progress"

ERIN = "erin@example.com"


def test_tasks_stale_in_progress(spec: SpecRun) -> None:
    """A window that includes today serves erin's 5 stale issues, the department
    spread {1,2,3,4,5}, and a day bucket carrying the 5."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-01-01", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.stale_in_progress",
                        "views": [{"view": "period"}, {"view": "peer"}, {"view": "timeseries", "bucket": "day"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.stale_in_progress", "period", entity_id=ERIN).equals(value=5)
    r.row("tasks.stale_in_progress", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    points = one(r.series("tasks.stale_in_progress"), entity_id=ERIN)["points"]
    assert some(points, value=5.0)


def test_tasks_stale_in_progress_out_of_window(spec: SpecRun) -> None:
    """A window that excludes today serves a null value."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "tasks.stale_in_progress", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.stale_in_progress", "period", entity_id=ERIN).equals(value=None)
