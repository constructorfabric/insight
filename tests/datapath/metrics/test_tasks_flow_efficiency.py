"""Task-delivery flow efficiency, served per person with the department distribution.

Bronze: Jira issues closed after an Open -> In Progress -> Closed history and the
BambooHR roster that gives the Engineering cohort. Silver reconstructs the status
intervals; gold serves 100 * In Progress time / lead time per person, the peer view
being the department distribution with max as the plain cohort max. Every issue has a
5-day lead and r days of dev time, so the department spreads {20,40,60,80,100}.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_flow_efficiency"

ERIN = "erin@example.com"


def test_tasks_flow_efficiency(spec: SpecRun) -> None:
    """Mar 20 .. Mar 31 takes every issue: erin's flow is 100, the department spreads
    {20,40,60,80,100}, and the Mar 25 close-day bucket carries her value."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.flow_efficiency",
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

    r.row("tasks.flow_efficiency", "period", entity_id=ERIN).equals(value=100)
    r.row("tasks.flow_efficiency", "peer", entity_id=ERIN).equals(
        target_value=100, p25=40, median=60, p75=80, min=20, max=100, n=5
    )
    points = one(r.series("tasks.flow_efficiency"), entity_id=ERIN)["points"]
    assert float(one(points, bucket_start="2026-03-25")["value"]) == 100.0


def test_tasks_flow_efficiency_empty_window(spec: SpecRun) -> None:
    """A window before any issue existed serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "tasks.flow_efficiency", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.flow_efficiency", "period", entity_id=ERIN).equals(value=None)
