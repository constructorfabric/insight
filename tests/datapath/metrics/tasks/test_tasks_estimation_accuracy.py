"""Task estimation accuracy per person, with the department distribution as the peer view.

Bronze: closed Jira issues whose changelog carries the close transition together with
timeoriginalestimate and timespent (neither is on the issue snapshot), plus BambooHR
employees placing the five members in one Engineering department. Gold serves
accuracy = 100 - |100 - 100 * estimate / spent|, clamped, per person over the window;
the peer view is the department distribution, and an empty window serves null.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import approx, one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_estimation_accuracy"

ERIN = "erin@example.com"


def test_tasks_estimation_accuracy(spec: SpecRun) -> None:
    """Each member closes one issue on Dec 25 with 10h spent and a 6h..10h estimate, so
    the department spreads {60,70,80,90,100} and erin's is 100."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-20", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.estimation_accuracy",
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

    r.row("tasks.estimation_accuracy", "period", entity_id=ERIN).equals(value=100)
    r.row("tasks.estimation_accuracy", "peer", entity_id=ERIN).equals(
        target_value=100, p25=70, median=80, p75=90, min=60, max=100, n=5
    )
    points = one(r.series("tasks.estimation_accuracy"), entity_id=ERIN)["points"]
    assert float(one(points, bucket_start="2026-12-25")["value"]) == approx(100.0)


def test_tasks_estimation_accuracy_empty_window(spec: SpecRun) -> None:
    """A window with no closed issues serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [
                    {"metric_key": "tasks.estimation_accuracy", "views": [{"view": "period"}]}
                ],
            },
        }
    )
    assert r.status == 200
    r.row("tasks.estimation_accuracy", "period", entity_id=ERIN).equals(value=None)
