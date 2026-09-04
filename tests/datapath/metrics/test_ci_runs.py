"""Decided CI pipeline runs counted at tenant grain, whatever the trigger.

Bronze: GitHub workflow run attempts. Silver: attempts dedup, so a run counts once at
its last attempt. Gold counts every decided run across all triggers while in-flight
runs wait. Seeded: a push success, a schedule success and a pull_request cancellation,
three decided runs across three triggers, so the period total is 3 and the per-trigger
breakdown is 1 each.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ci_runs"

TENANT = "11111111-1111-1111-1111-111111111111"


def test_every_decided_run_counts_once_whatever_the_trigger(spec: SpecRun) -> None:
    """Three decided runs across push, schedule and pull_request: period 3, the Mar 01
    daily point 3, and the schedule trigger 1 in the breakdown."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": {"from": "2026-03-01", "to": "2026-03-07"},
                "metrics": [
                    {
                        "metric_key": "ci.runs",
                        "views": [
                            {"view": "period"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["trigger"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ci.runs", "period", entity_id=TENANT).equals(value=3.0)
    assert any(
        some(series["points"], bucket_start="2026-03-01", value=3.0)
        for series in r.series("ci.runs")
    )
    assert some(
        r.breakdown("ci.runs"), dimensions={"key": "trigger", "value": "schedule"}, value=1.0
    )
