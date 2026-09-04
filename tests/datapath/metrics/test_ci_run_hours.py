"""Wall-clock hours spent running CI, served at tenant grain.

Bronze: workflow run attempts with start and finish timestamps. Silver: duration is
finish minus start of the last attempt. Gold sums hours over all decided runs, any
trigger; outcome rides as a dimension so red hours are a filter, not another metric.
Seeded: a 30-minute success and a 90-minute failure, 2.0 hours total and 1.5 red.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ci_run_hours"


def test_ci_hours_sum_wall_clock_and_split_red_by_outcome(spec: SpecRun) -> None:
    """The period sums both runs to 2.0 hours, the daily point carries the same, and the
    outcome breakdown isolates the 1.5 red hours."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": {"from": "2026-03-01", "to": "2026-03-07"},
                "metrics": [
                    {
                        "metric_key": "ci.run_hours",
                        "views": [
                            {"view": "period"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["outcome"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ci.run_hours", "period", entity_id="11111111-1111-1111-1111-111111111111").equals(
        value=2.0
    )
    points = [p for s in r.series("ci.run_hours") for p in s["points"]]
    assert some(points, bucket_start="2026-03-02", value=2.0)
    assert some(
        r.breakdown("ci.run_hours"), dimensions={"key": "outcome", "value": "failure"}, value=1.5
    )
