"""Mean time to resolution per person from Jira, with the department distribution.

Bronze: five closed Jira issues with one In Progress -> Closed history event each,
their Jira users, and BambooHR employees giving the Engineering cohort. Enrichment
reconstructs the created-to-close lead time; gold serves each member's own mean
resolution days, the peer view the five-member distribution. An empty window is null.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one, some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_resolution_time"

ERIN = "erin@example.com"


def test_tasks_resolution_time(spec: SpecRun) -> None:
    """Erin's one issue took 5 days; the department ladder {1,2,3,4,5} gives p25=2,
    median=3, p75=4, and the single Jira instance makes the source breakdown one row."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-20", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.resolution_time",
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

    r.row("tasks.resolution_time", "period", entity_id=ERIN).equals(value=5)
    r.row("tasks.resolution_time", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    points = one(r.series("tasks.resolution_time"), entity_id=ERIN)["points"]
    assert some(points, value=5.0)
    assert len(one(r.histogram("tasks.resolution_time"), entity_id=ERIN)["bins"]) > 0
    by_source = one(
        r.breakdown("tasks.resolution_time"),
        entity_id=ERIN,
        dimensions={"key": "source", "value": "jira"},
    )
    assert float(by_source["value"]) == 5.0
    assert len(some(r.breakdown("tasks.resolution_time"), entity_id=ERIN)) == 1


def test_tasks_resolution_time_by_issue_type(spec: SpecRun) -> None:
    """The type breakdown is a single Task row carrying erin's own 5 days."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-20", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.resolution_time",
                        "views": [{"view": "breakdown", "dimensions": ["type"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    by_type = one(
        r.breakdown("tasks.resolution_time"),
        entity_id=ERIN,
        dimensions={"key": "type", "value": "Task"},
    )
    assert float(by_type["value"]) == 5.0
    assert len(some(r.breakdown("tasks.resolution_time"), entity_id=ERIN)) == 1


def test_tasks_resolution_time_empty_window(spec: SpecRun) -> None:
    """A window with no closed issues serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "tasks.resolution_time", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.resolution_time", "period", entity_id=ERIN).equals(value=None)
