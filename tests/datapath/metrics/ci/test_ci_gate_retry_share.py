"""Share of gate runs whose final state came from a re-run, served at tenant grain.

Bronze: workflow run attempts, where a re-run arrives as a higher run_attempt.
Silver counts a run once, at its last attempt; retried means that attempt is above 1.
Gold serves retried gate runs over gate runs. Four seeded gate runs, one of them on
attempt 2, give 25% for the period, the daily point and the per-pipeline breakdown.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ci_gate_retry_share"


def test_retry_share_counts_a_run_once_at_its_last_attempt(spec: SpecRun) -> None:
    """Four gate runs, one on its second attempt: 25% for the period, the Mar 01 day
    and the ci.yml pipeline."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": {"from": "2026-03-01", "to": "2026-03-07"},
                "metrics": [
                    {
                        "metric_key": "ci.gate_retry_share",
                        "views": [
                            {"view": "period"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["pipeline"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ci.gate_retry_share", "period", entity_id=spec.tenant).equals(value=25.0)
    points = [p for s in r.series("ci.gate_retry_share") for p in s["points"]]
    assert some(points, bucket_start="2026-03-01", value=25.0)
    assert some(
        r.breakdown("ci.gate_retry_share"),
        dimensions={"key": "pipeline", "value": ".github/workflows/ci.yml"},
        value=25.0,
    )
