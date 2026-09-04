"""Task Delivery: issues closed via GitHub, split by issue type and by source.

Bronze: GitHub issues, their ClosedEvent timeline rows (the close date) and issue
types, plus the configuration rows that bind `closed:completed` and `closed:not_planned`
to a lifecycle category. Five department members of rank r each close r issues on one
close date inside the custom window, so the peer distribution is {1,2,3,4,5}; erin's five
split into 4 Task + 1 Bug, every row carries `source: github`, a not_planned closure
still counts, a re-extracted duplicate row changes nothing, and dev time is absent
because GitHub has no in-progress category.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import one, some
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "github_tasks_closed"

ERIN = "erin@example.com"


def test_tasks_closed_via_github(spec: SpecRun) -> None:
    """Erin's five closures, their type split, the department distribution at the
    disclosure floor, and no dev time for a tracker without an in-progress category."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.closed",
                        "views": [{"view": "period"}, {"view": "peer"}, {"view": "breakdown", "dimensions": ["type"]}],
                    },
                    {"metric_key": "tasks.bugs_fixed", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.closed_non_bug", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.dev_time", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.closed", "period", entity_id=ERIN).equals(value=5)
    r.row("tasks.closed", "peer", entity_id=ERIN).equals(target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5)
    by_type = r.breakdown("tasks.closed")
    assert float(one(by_type, entity_id=ERIN, dimensions={"key": "type", "value": "Task"})["value"]) == 4.0
    assert float(one(by_type, entity_id=ERIN, dimensions={"key": "type", "value": "Bug"})["value"]) == 1.0

    r.row("tasks.bugs_fixed", "period", entity_id=ERIN).equals(value=1)
    r.row("tasks.closed_non_bug", "period", entity_id=ERIN).equals(value=4)

    dev_time = some(r.rows("tasks.dev_time", "period"), entity_id=ERIN)
    assert [row for row in dev_time if row.get("value") is not None] == []


def test_tasks_closed_split_by_source(spec: SpecRun) -> None:
    """The source dimension carries every closure under github and nothing else."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [{"metric_key": "tasks.closed", "views": [{"view": "breakdown", "dimensions": ["source"]}]}],
            },
        }
    )
    assert r.status == 200

    by_source = r.breakdown("tasks.closed")
    assert float(one(by_source, entity_id=ERIN, dimensions={"key": "source", "value": "github"})["value"]) == 5.0
    assert len(some(by_source, entity_id=ERIN)) == 1
