"""Median diff size (lines added plus removed) of a person's authored commits.

Bronze: GitHub commits with line stats; a merge commit with a huge diff must
contribute nothing. Erin's sizes over the window are {4, 6, 10, 30, 204}, so only
the median fits the period value; each day and each repository buckets its own
median, and the histogram bins span erin's own [4, 204]. A commit whose source
reported no stats enters the pool as 0 rather than abstaining.
"""

from __future__ import annotations

import pytest
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_commit_size"

DAVE = "dave@example.com"
ERIN = "erin@example.com"


def test_the_period_value_is_the_exact_median_of_the_per_commit_sizes(spec: SpecRun) -> None:
    """{4, 6, 10, 30, 204} reads 10; each day and repository serves its own median,
    and ten fixed-width bins over [4, 204] put {4, 6, 10} first and 204 last."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.commit_size",
                        "views": [
                            {"view": "period"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["repository"]},
                            {"view": "histogram"},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commit_size", "period", entity_id=ERIN).equals(value=10)
    r.row("git.commit_size", "timeseries", entity_id=ERIN).contains(points={"bucket_start": "2026-10-01", "value": 30})
    r.row("git.commit_size", "timeseries", entity_id=ERIN).contains(points={"bucket_start": "2026-10-02", "value": 6})
    r.row(
        "git.commit_size", "breakdown", entity_id=ERIN, dimensions={"key": "repository", "value": "git-test:acme/api"}
    ).equals(value=30)
    r.row(
        "git.commit_size", "breakdown", entity_id=ERIN, dimensions={"key": "repository", "value": "git-test:acme/web"}
    ).equals(value=6)
    r.row("git.commit_size", "histogram", entity_id=ERIN).contains(bins={"lo": 4, "count": 3})
    r.row("git.commit_size", "histogram", entity_id=ERIN).contains(bins={"hi": 204, "count": 1})


def test_a_commit_whose_source_reported_no_stats_reads_zero_not_absent(spec: SpecRun) -> None:
    """Staging projects absent stats to zero, so dave's one commit reads a size of 0, not null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [DAVE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "period"}]},
                    {"metric_key": "git.commit_size", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commits", "period", entity_id=DAVE).equals(value=1)
    r.row("git.commit_size", "period", entity_id=DAVE).equals(value=0)
