"""Typical (median) minutes a gate run takes, served at tenant grain.

Bronze: workflow run attempts with start and finish timestamps. Silver: the
duration is finish minus start of the last attempt, in minutes. Gold serves
per-run rows at event grain; the value is the exact median over the runs in the
period, never the mean. Three seeded runs of 1, 2 and 10 minutes give a median of
2 against a mean of 4.33, so a mean-computed value fails.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ci_run_duration_min"

TENANT = "11111111-1111-1111-1111-111111111111"


def test_duration_is_the_median_over_runs_not_the_mean(spec: SpecRun) -> None:
    """The period value is the median 2, the daily point and the per-pipeline
    breakdown agree, and the histogram exposes the event rows."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": {"from": "2026-03-01", "to": "2026-03-07"},
                "metrics": [
                    {
                        "metric_key": "ci.run_duration_min",
                        "views": [
                            {"view": "period"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["pipeline"]},
                            {"view": "histogram"},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ci.run_duration_min", "period", entity_id=TENANT).equals(value=2.0)
    points = [p for s in r.series("ci.run_duration_min") for p in s["points"]]
    assert some(points, bucket_start="2026-03-02", value=2.0)
    assert some(
        r.breakdown("ci.run_duration_min"),
        dimensions={"key": "pipeline", "value": ".github/workflows/ci.yml"},
        value=2.0,
    )
    r.row("ci.run_duration_min", "histogram", entity_id=TENANT).nonempty("bins")
