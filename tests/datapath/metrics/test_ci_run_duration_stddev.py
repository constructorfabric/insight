"""Sample standard deviation of gate run duration in minutes, served at tenant grain.

Bronze: GitHub workflow run attempts with start and finish timestamps. Silver: the
duration of each run's last attempt, in minutes. Gold keeps per-run duration rows at
event grain and serves the sample stddev over the runs in the period, org-wide. Eight
seeded runs of [2, 4, 4, 4, 5, 5, 7, 9] minutes spread sqrt(32/7), about 2.138; a
single-run window has no measurable spread and reads null, never a fabricated zero.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ci_run_duration_stddev"


def test_duration_spread_is_the_sample_stddev_over_runs(spec: SpecRun) -> None:
    """Eight runs in Mar 01 .. Mar 07 spread sqrt(32/7), pinned to (2.1, 2.2) in the
    period, the Mar 02 day bucket and the acme/app repository breakdown."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": {"from": "2026-03-01", "to": "2026-03-07"},
                "metrics": [
                    {
                        "metric_key": "ci.run_duration_min_stddev",
                        "views": [
                            {"view": "period"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["repository"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ci.run_duration_min_stddev", "period", entity_id=spec.tenant).check(
        "value", lambda v: 2.1 < float(v) < 2.2, "2.1 < value < 2.2"
    )

    march_second = some(
        [p for s in r.series("ci.run_duration_min_stddev") for p in s["points"]],
        bucket_start="2026-03-02",
    )
    assert any(2.1 < float(p["value"]) < 2.2 for p in march_second if p["value"] is not None), (
        march_second
    )

    by_repository = some(
        r.breakdown("ci.run_duration_min_stddev"),
        dimensions={"key": "repository", "value": "acme/app"},
    )
    assert any(2.1 < float(v["value"]) < 2.2 for v in by_repository if v["value"] is not None), (
        by_repository
    )


def test_a_single_run_has_no_measurable_spread_and_reads_null(spec: SpecRun) -> None:
    """The Apr 01 .. Apr 07 window holds one run; the stddev is null, not zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": {"from": "2026-04-01", "to": "2026-04-07"},
                "metrics": [
                    {"metric_key": "ci.run_duration_min_stddev", "views": [{"view": "period"}]}
                ],
            },
        }
    )
    assert r.status == 200
    r.row("ci.run_duration_min_stddev", "period", entity_id=spec.tenant).equals(value=None)
