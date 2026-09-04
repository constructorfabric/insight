"""Task pickup time per person, with the department distribution as the peer view.

Bronze: Jira issues, their status history (Open -> In Progress -> Closed) and users;
BambooHR employees give the Engineering cohort. Enrich reconstructs status intervals;
pickup time is the days an issue waited in Open before its first In Progress move.
Five members hold one issue each, picked up after 1..5 days, so erin's value is 5 and
the department spreads {1,2,3,4,5}; a window with no issues serves null.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one, some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_pickup_time"

ERIN = "erin@example.com"


def test_tasks_pickup_time(spec: SpecRun) -> None:
    """Erin's pickup is 5 days; the department ladder gives median 3, p25 2, p75 4,
    range 1..5; the single Jira instance makes the source breakdown one row."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.pickup_time",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "histogram"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.pickup_time", "period", entity_id=ERIN).equals(value=5)
    r.row("tasks.pickup_time", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    assert some(one(r.series("tasks.pickup_time"), entity_id=ERIN)["points"], value=5.0)
    assert len(one(r.histogram("tasks.pickup_time"), entity_id=ERIN)["bins"]) > 0
    by_source = one(
        r.breakdown("tasks.pickup_time"),
        entity_id=ERIN,
        dimensions={"key": "source", "value": "jira"},
    )
    assert float(by_source["value"]) == 5.0
    assert len(some(r.breakdown("tasks.pickup_time"), entity_id=ERIN)) == 1


def test_tasks_pickup_time_by_issue_type(spec: SpecRun) -> None:
    """Erin's one issue is a Task, so the type breakdown is a single row carrying 5."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.pickup_time",
                        "views": [{"view": "breakdown", "dimensions": ["type"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    by_type = one(
        r.breakdown("tasks.pickup_time"),
        entity_id=ERIN,
        dimensions={"key": "type", "value": "Task"},
    )
    assert float(by_type["value"]) == 5.0
    assert len(some(r.breakdown("tasks.pickup_time"), entity_id=ERIN)) == 1


def test_tasks_pickup_time_empty_window(spec: SpecRun) -> None:
    """A window with no issues serves null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "tasks.pickup_time", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.pickup_time", "period", entity_id=ERIN).equals(value=None)
