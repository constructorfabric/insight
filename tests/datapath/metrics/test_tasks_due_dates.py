"""Task due-date compliance, on-time delivery and average slip, served per person.

Bronze: Jira issues carrying a due date, closed by a Status -> Closed changelog
event, and the Jira users assigned to them. Silver counts, per person and close-day,
the closed tasks with a due date and those closed on time. Gold serves the requested
member's own compliance, and the peer view is the department-of-5 distribution. The
due date reaches gold only through the bronze `due_date` column; an empty window
serves null.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_due_dates"

ERIN = "erin@example.com"
CLOSE_DAY = "2026-06-25"


def test_unified_task_due_date_metrics(spec: SpecRun) -> None:
    """Jun 20 .. Jun 30 takes the one close day: erin's compliance and on-time delivery
    are 75, her average slip is 5, and each peer view carries the department stats."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-06-20", "to": "2026-06-30"},
                "metrics": [
                    {
                        "metric_key": "tasks.due_date_compliance",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    },
                    {
                        "metric_key": "tasks.on_time_delivery",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    },
                    {
                        "metric_key": "tasks.avg_slip",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.due_date_compliance", "period", entity_id=ERIN).equals(value=75)
    r.row("tasks.due_date_compliance", "peer", entity_id=ERIN).equals(
        target_value=75, p25=None, median=None, p75=None, min=None, max=None, n=4
    )
    compliance = one(r.series("tasks.due_date_compliance"), entity_id=ERIN)["points"]
    assert float(one(compliance, bucket_start=CLOSE_DAY)["value"]) == 75.0

    r.row("tasks.on_time_delivery", "period", entity_id=ERIN).equals(value=75)
    r.row("tasks.on_time_delivery", "peer", entity_id=ERIN).equals(
        target_value=75, p25=None, median=None, p75=None, min=None, max=None, n=4
    )
    on_time = one(r.series("tasks.on_time_delivery"), entity_id=ERIN)["points"]
    assert float(one(on_time, bucket_start=CLOSE_DAY)["value"]) == 75.0

    r.row("tasks.avg_slip", "period", entity_id=ERIN).equals(value=5)
    r.row("tasks.avg_slip", "peer", entity_id=ERIN).equals(
        target_value=5, p25=5, median=5, p75=5, min=5, max=5, n=5
    )
    slip = one(r.series("tasks.avg_slip"), entity_id=ERIN)["points"]
    assert float(one(slip, bucket_start=CLOSE_DAY)["value"]) == 5.0


def test_unified_task_due_date_metrics_empty_window(spec: SpecRun) -> None:
    """A window with no closed tasks serves null for all three metrics."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [
                    {"metric_key": "tasks.due_date_compliance", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.on_time_delivery", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.avg_slip", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.due_date_compliance", "period", entity_id=ERIN).equals(value=None)
    r.row("tasks.on_time_delivery", "period", entity_id=ERIN).equals(value=None)
    r.row("tasks.avg_slip", "period", entity_id=ERIN).equals(value=None)
