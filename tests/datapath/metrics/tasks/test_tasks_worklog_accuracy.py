"""Worklog logging accuracy per person, served over a custom window.

Bronze: Jira issues closed after exactly one In Progress day, their changelog (the
in-progress seconds), the worklogs (the logged seconds) and the users; BambooHR
employees give the Engineering cohort. Gold serves least(100, 100 * logged /
in-progress) per person; the peer view is the department distribution, and a window
with no rows serves null.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_worklog_accuracy"

ERIN = "erin@example.com"


def test_tasks_worklog_accuracy(spec: SpecRun) -> None:
    """Mar 20 .. Mar 31 takes the one-day issues; each member logs r*17280 s of the
    86400 s in progress, so the department spreads {20,40,60,80,100} and erin's is 100."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.worklog_accuracy",
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

    r.row("tasks.worklog_accuracy", "period", entity_id=ERIN).equals(value=100)
    r.row("tasks.worklog_accuracy", "peer", entity_id=ERIN).equals(
        target_value=100, p25=40, median=60, p75=80, min=20, max=100, n=5
    )
    points = one(r.series("tasks.worklog_accuracy"), entity_id=ERIN)["points"]
    assert any(float(p["value"]) == 100.0 for p in points if p["value"] is not None)


def test_tasks_worklog_accuracy_empty_window(spec: SpecRun) -> None:
    """A window with no rows serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [
                    {"metric_key": "tasks.worklog_accuracy", "views": [{"view": "period"}]}
                ],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.worklog_accuracy", "period", entity_id=ERIN).equals(value=None)
