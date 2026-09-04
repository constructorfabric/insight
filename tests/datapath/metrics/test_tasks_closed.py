"""Task Delivery: issues closed and their split by issue type, from JIRA, served per
person over a custom window with the department-of-5 distribution.

Bronze: closed issues, the Status -> Closed history event that dates the close, the
users and the issue types. Gold counts every closed issue regardless of type and
carries the type as a breakdown dimension; a type neither configured list covers
stays its own group. An empty window serves null.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import approx, one, some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_closed"

ERIN = "erin@example.com"


def test_tasks_closed(spec: SpecRun) -> None:
    """Dec 20 .. Dec 31 takes all five ranks; erin's 5 closed split 3 Task + 1 Bug +
    1 Incident, and only the Bug and the Tasks land in the configured-list metrics."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-20", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.closed",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["type"]},
                        ],
                    },
                    {"metric_key": "tasks.bugs_fixed", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.closed_non_bug", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.closed", "period", entity_id=ERIN).equals(value=5)
    r.row("tasks.closed", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    closed = one(r.series("tasks.closed"), entity_id=ERIN)["points"]
    assert float(one(closed, bucket_start="2026-12-25")["value"]) == approx(5.0)
    by_type = r.breakdown("tasks.closed")
    assert (
        float(one(by_type, entity_id=ERIN, dimensions={"key": "type", "value": "Task"})["value"])
        == 3.0
    )
    assert (
        float(one(by_type, entity_id=ERIN, dimensions={"key": "type", "value": "Bug"})["value"])
        == 1.0
    )
    assert (
        float(
            one(by_type, entity_id=ERIN, dimensions={"key": "type", "value": "Incident"})["value"]
        )
        == 1.0
    )
    assert len(some(by_type, entity_id=ERIN)) == 3

    r.row("tasks.bugs_fixed", "period", entity_id=ERIN).equals(value=1)
    r.row("tasks.closed_non_bug", "period", entity_id=ERIN).equals(value=3)


def test_tasks_closed_empty_window(spec: SpecRun) -> None:
    """A window with no closed issues serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "tasks.closed", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.closed", "period", entity_id=ERIN).equals(value=None)
