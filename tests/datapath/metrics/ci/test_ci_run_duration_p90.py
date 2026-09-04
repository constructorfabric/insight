"""The 90th percentile of gate run duration in minutes, served at tenant grain.

Bronze: workflow run attempts with start and finish timestamps. Silver: duration is
finish minus start of the last attempt, in minutes; only decided gate runs carry one.
Gold keeps per-run duration rows at event grain and serves p90 as the exact 0.9-quantile
over the period's runs, org-wide. Nine runs of 1..9 minutes pin p90 to [8, 9], so an
off-by-one rank fails while the test stays engine-honest; the histogram exposes the rows.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ci_run_duration_p90"


def test_p90_duration_reads_the_tail_of_the_run_distribution(spec: SpecRun) -> None:
    """Nine runs of 1..9 minutes: the exact 0.9-quantile lands in [8, 9] for the period,
    the Mar 02 day bucket and the ci.yml pipeline; the histogram exposes the event rows."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": {"from": "2026-03-01", "to": "2026-03-07"},
                "metrics": [
                    {
                        "metric_key": "ci.run_duration_min_p90",
                        "views": [
                            {"view": "period"},
                            {"view": "histogram"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["pipeline"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ci.run_duration_min_p90", "period", entity_id=spec.tenant).check(
        "value", lambda v: 8.0 <= float(v) <= 9.0, "8 <= value <= 9"
    )
    r.row("ci.run_duration_min_p90", "histogram", entity_id=spec.tenant).nonempty("bins")

    day = some(
        [p for s in r.series("ci.run_duration_min_p90") for p in s["points"]],
        bucket_start="2026-03-02",
    )
    assert any(8.0 <= float(p["value"]) <= 9.0 for p in day if p["value"] is not None)

    by_pipeline = some(
        r.breakdown("ci.run_duration_min_p90"),
        dimensions={"key": "pipeline", "value": ".github/workflows/ci.yml"},
    )
    assert any(8.0 <= float(v["value"]) <= 9.0 for v in by_pipeline if v["value"] is not None)
