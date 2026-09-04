"""Time to first review (p75), served per person over a window.

Bronze: GitHub pull requests with created_at, reviews with submitted_at, and per-request
diff stats carrying the author e-mail. Silver: `class_git_pull_requests` and
`class_git_pull_requests_reviewers`. Gold dates one event per reviewed request at its
EARLIEST review, in hours from open; serve takes quantileExactIf(0.75) over the period's
events, so a later second review on the same request never moves the value.
"""

from __future__ import annotations

import pytest
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_first_review_time_p75"

ALICE = "alice@example.com"


def test_slow_end_first_review_time_across_views(spec: SpecRun) -> None:
    """alice's five requests get first reviews 10, 20, 30, 40 and 100 hours after opening;
    quantileExact(0.75) picks index floor(0.75 x 5) = 3 -> 40, and the department spreads
    {10, 20, 30, 40, 50}."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-10-01", "to": "2026-10-06"},
                "metrics": [
                    {
                        "metric_key": "git.first_review_time_p75_h",
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

    r.row("git.first_review_time_p75_h", "period", entity_id=ALICE).equals(value=40)
    r.row("git.first_review_time_p75_h", "peer", entity_id=ALICE).equals(
        target_value=40, p25=20, median=30, p75=40, min=10, max=50, n=5
    )
    series = r.row("git.first_review_time_p75_h", "timeseries", entity_id=ALICE)
    series.contains(points={"bucket_start": "2026-10-01", "value": 20})
    series.contains(points={"bucket_start": "2026-10-05", "value": 100})
    r.row(
        "git.first_review_time_p75_h", "breakdown", entity_id=ALICE, dimensions={"key": "source", "value": "github"}
    ).equals(value=40)
    r.row("git.first_review_time_p75_h", "histogram", entity_id=ALICE).nonempty("bins")


def test_empty_window(spec: SpecRun) -> None:
    """A window with no reviewed requests serves an honest null, not a zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "git.first_review_time_p75_h", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.first_review_time_p75_h", "period", entity_id=ALICE).equals(value=None)
