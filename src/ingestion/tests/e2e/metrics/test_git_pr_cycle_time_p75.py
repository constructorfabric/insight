"""PR cycle time at the 75th percentile, served per person over a window.

Bronze: pull requests with created_at/merged_at, the author e-mail carried by the
per-request diff stats. Silver: one `class_git_pull_requests` row per request. Gold:
hours from open to merge, one event per merged request dated at the merge day, served
as quantileExactIf(0.75) over the period's events. alice merges five requests with cycle
hours [10, 20, 30, 40, 100]; the p75 is the sorted element at index floor(0.75 x 5) = 40.
"""

from __future__ import annotations

import pytest
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_pr_cycle_time_p75"

ALICE = "alice@example.com"


def test_slow_end_cycle_time_across_views(spec: SpecRun) -> None:
    """quantileExact(0.75) of [10, 20, 30, 40, 100] is 40, not the median 30; the
    2026-10-01 bucket holds [10, 20] and serves 20, 2026-10-05 holds [100] alone."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-10-01", "to": "2026-10-06"},
                "metrics": [
                    {
                        "metric_key": "git.pr_cycle_time_p75_h",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                            {"view": "histogram"},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.pr_cycle_time_p75_h", "period", entity_id=ALICE).equals(value=40)
    r.row("git.pr_cycle_time_p75_h", "peer", entity_id=ALICE).equals(
        target_value=40, p25=20, median=30, p75=40, min=10, max=50, n=5
    )
    series = r.row("git.pr_cycle_time_p75_h", "timeseries", entity_id=ALICE)
    series.contains(points={"bucket_start": "2026-10-01", "value": 20})
    series.contains(points={"bucket_start": "2026-10-05", "value": 100})
    r.row(
        "git.pr_cycle_time_p75_h", "breakdown", entity_id=ALICE, dimensions={"key": "source", "value": "github"}
    ).equals(value=40)
    r.row("git.pr_cycle_time_p75_h", "histogram", entity_id=ALICE).nonempty("bins")


def test_empty_window(spec: SpecRun) -> None:
    """A window with no merged requests serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "git.pr_cycle_time_p75_h", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_cycle_time_p75_h", "period", entity_id=ALICE).equals(value=None)
