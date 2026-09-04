"""Commits the git connector collected, dated at their commit time, served at tenant grain.

Bronze: the commits stream. Silver: the commit class accumulates. Gold serves one
event row per collected commit, dated at its commit time -- the denominator of the
run-to-commit join-coverage panel. Two commits on different days give a period
total of 2 and one daily point each.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import some
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ci_commits_observed"

TENANT = "11111111-1111-1111-1111-111111111111"


def test_every_collected_commit_counts_once_at_its_commit_date(spec: SpecRun) -> None:
    """Two commits on Mar 02 and Mar 03 make a period total of 2 and one point per day."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": {"from": "2026-03-01", "to": "2026-03-07"},
                "metrics": [
                    {
                        "metric_key": "ci.commits_observed",
                        "views": [{"view": "period"}, {"view": "timeseries", "bucket": "day"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ci.commits_observed", "period", entity_id=TENANT).equals(value=2.0)
    daily = [s["points"] for s in r.series("ci.commits_observed")]
    assert any(
        some(points, bucket_start="2026-03-02", value=1.0) and some(points, bucket_start="2026-03-03", value=1.0)
        for points in daily
    )
