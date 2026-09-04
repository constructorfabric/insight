"""Task dev time: hours a closed issue spent in dev-active statuses, served per person.

Bronze: Jira issues, their status history (In Progress -> Closed) and users; BambooHR
employees give the Engineering cohort. Silver turns the per-field event log into status
intervals and sums the dev-active seconds per closed issue. Gold serves each person's
median hours; the peer view is the department distribution, whose range_max is the
cohort P95 and equals the max on a five-point ladder. One Jira instance means a single
`source` breakdown row carrying the member's own figure; an empty window serves null.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import approx, one, some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_dev_time"

ERIN = "erin@example.com"


def test_tasks_dev_time(spec: SpecRun) -> None:
    """Erin (rank 5) over Mar 20 .. Mar 31: value 5 h against the department ladder
    {1,2,3,4,5}; the single Jira source row carries her own figure."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.dev_time",
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

    r.row("tasks.dev_time", "period", entity_id=ERIN).equals(value=5)
    r.row("tasks.dev_time", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    points = one(r.series("tasks.dev_time"), entity_id=ERIN)["points"]
    assert float(one(points, bucket_start="2026-03-25")["value"]) == approx(5.0)
    assert len(one(r.histogram("tasks.dev_time"), entity_id=ERIN)["bins"]) > 0
    by_source = one(
        r.breakdown("tasks.dev_time"), entity_id=ERIN, dimensions={"key": "source", "value": "jira"}
    )
    assert float(by_source["value"]) == approx(5.0)
    assert len(some(r.breakdown("tasks.dev_time"), entity_id=ERIN)) == 1


def test_tasks_dev_time_by_issue_type(spec: SpecRun) -> None:
    """The `type` breakdown is a single Task row carrying erin's own 5 h."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.dev_time",
                        "views": [{"view": "breakdown", "dimensions": ["type"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    by_type = one(
        r.breakdown("tasks.dev_time"), entity_id=ERIN, dimensions={"key": "type", "value": "Task"}
    )
    assert float(by_type["value"]) == approx(5.0)
    assert len(some(r.breakdown("tasks.dev_time"), entity_id=ERIN)) == 1


def test_tasks_dev_time_empty_window(spec: SpecRun) -> None:
    """A window with no closed issues serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "tasks.dev_time", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.dev_time", "period", entity_id=ERIN).equals(value=None)
