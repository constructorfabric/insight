"""Share of commit-triggered CI runs that passed, served at tenant grain.

Bronze: GitHub Actions workflow runs, one row per run attempt, re-sync duplicates
possible. Silver dedups attempts on unique_key and stamps the gate once: a run
triggered by push, pull_request or merge_group whose outcome decided (success,
failure or timeout); approval-walled, cancelled and skipped runs are not decided.
Gold counts a retried run once at its last attempt; pass rate is passed gate runs
over gate runs, org-wide.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ci_gate_pass_rate"


def test_dedups_gates_and_counts_merge_queue_and_retried_runs_in_the_gate(spec: SpecRun) -> None:
    """Eight gate runs, four passed: the re-sync duplicate counts once, merge-queue runs sit
    on both sides of the ratio, and a retried run counts at its last attempt."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": {"from": "2026-03-01", "to": "2026-03-07"},
                "metrics": [
                    {
                        "metric_key": "ci.gate_pass_rate",
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

    r.row("ci.gate_pass_rate", "period", entity_id=spec.tenant).equals(value=50.0)
    points = [p for s in r.series("ci.gate_pass_rate") for p in s["points"]]
    assert some(points, bucket_start="2026-03-01", value=50.0)
    assert some(
        r.breakdown("ci.gate_pass_rate"),
        dimensions={"key": "repository", "value": "acme/app"},
        value=50.0,
    )


def test_gate_pass_rate_empty_window_is_null_not_zero(spec: SpecRun) -> None:
    """A window with no runs serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "tenant"},
                "period": {"from": "2026-05-01", "to": "2026-05-07"},
                "metrics": [{"metric_key": "ci.gate_pass_rate", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("ci.gate_pass_rate", "period", entity_id=spec.tenant).equals(value=None)
