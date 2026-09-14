"""Unified Git metrics: authored commits, code line categories, pull requests, cohort
merge rate, medians and histograms, served per person over a window.

Bronze: GitHub commits with their file changes, pull requests with diff stats and
reviews. Commit sizes are additions plus deletions (12, 24, 36, 48, 60); erin's second
commit, collected without file changes, is 50. Repository rollups pool observations
across people, and an empty window serves null.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_metrics"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"
DAVE = "dave@example.com"
ERIN = "erin@example.com"
FRANK = "frank@example.com"
GRACE = "grace@example.com"

SOURCE_GITHUB = {"key": "source", "value": "github"}
CATEGORY_CODE = {"key": "category", "value": "code"}
REPOSITORY_INSIGHT = {"key": "repository", "value": "git-test:constructor/insight"}


def test_reviews_submitted_after_merge_do_not_contribute_wait_share(spec: SpecRun) -> None:
    """Grace's only review lands after her merge, so her wait share is null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [GRACE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [{"metric_key": "git.review_wait_share", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.review_wait_share", "period", entity_id=GRACE).equals(value=None)


def test_repository_rollups_pool_observations_across_people(spec: SpecRun) -> None:
    """One repository row per metric: merge rate over all six authors, cycle time over the five merged."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE, BOB, CAROL, DAVE, ERIN, FRANK]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.merge_rate",
                        "views": [{"view": "rollup", "dimensions": ["repository"]}],
                    },
                    {
                        "metric_key": "git.pr_cycle_time_h",
                        "views": [{"view": "rollup", "dimensions": ["repository"]}],
                    },
                ],
            },
        }
    )
    assert r.status == 200
    r.row("git.merge_rate", "rollup", dimensions=REPOSITORY_INSIGHT).equals(
        value=83.33333333333333, contributing_entity_count=6
    )
    r.row("git.pr_cycle_time_h", "rollup", dimensions=REPOSITORY_INSIGHT).equals(
        value=3, contributing_entity_count=5
    )


def test_merge_rate_is_zero_when_created_pull_requests_never_merge(spec: SpecRun) -> None:
    """Frank's one pull request is still open, so his merge rate is 0 rather than null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [FRANK]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [{"metric_key": "git.merge_rate", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.merge_rate", "period", entity_id=FRANK).equals(value=0)


def test_unified_git_metrics(spec: SpecRun) -> None:
    """Erin's day: two commits sized 50 and 60 (the median is the upper middle, 60), one merged
    PR with two reviewers; lines_added counts the fileless second commit, code_lines does not."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.commits",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.code_lines",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.lines_added",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["category", "source"]},
                        ],
                    },
                    {
                        "metric_key": "git.lines_removed",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["category", "source"]},
                        ],
                    },
                    {
                        "metric_key": "git.prs_created",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.prs_merged",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.merge_rate",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.commits_per_active_day",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["repository"]},
                        ],
                    },
                    {
                        "metric_key": "git.commit_size",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                            {"view": "histogram"},
                        ],
                    },
                    {
                        "metric_key": "git.pr_size",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                            {"view": "histogram"},
                        ],
                    },
                    {
                        "metric_key": "git.pr_cycle_time_h",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                            {"view": "histogram"},
                        ],
                    },
                    {
                        "metric_key": "git.test_change_share",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.pr_abandonment_rate",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.review_coverage",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.reviewers_per_pr",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.multi_reviewer_rate",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.merges_without_approval_rate",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.active_days",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["repository"]},
                        ],
                    },
                    {
                        "metric_key": "git.first_review_time_h",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                            {"view": "histogram"},
                        ],
                    },
                    {
                        "metric_key": "git.review_wait_share",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                            {"view": "histogram"},
                        ],
                    },
                    {
                        "metric_key": "git.review_to_merge_time_h",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                            {"view": "histogram"},
                        ],
                    },
                    {
                        "metric_key": "git.approval_to_merge_time_h",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                            {"view": "histogram"},
                        ],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commits", "period", entity_id=ERIN).equals(value=2)
    r.row("git.commits", "peer", entity_id=ERIN).equals(
        target_value=2, p25=1, median=1, p75=1, min=1, max=2, n=5
    )
    r.row("git.commits", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 2}
    )
    r.row("git.commits", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(value=2)

    r.row("git.code_lines", "period", entity_id=ERIN).equals(value=50)
    r.row("git.code_lines", "peer", entity_id=ERIN).equals(
        target_value=50, p25=20, median=30, p75=40, min=10, max=50, n=5
    )
    r.row("git.code_lines", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 50}
    )
    r.row("git.code_lines", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(value=50)

    r.row("git.lines_added", "period", entity_id=ERIN).equals(value=100)
    r.row("git.lines_added", "peer", entity_id=ERIN).equals(
        target_value=100, p25=20, median=30, p75=40, min=10, max=100, n=5
    )
    r.row("git.lines_added", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 100}
    )
    r.row("git.lines_added", "breakdown", entity_id=ERIN, dimensions=CATEGORY_CODE).equals(value=50)

    r.row("git.lines_removed", "period", entity_id=ERIN).equals(value=10)
    r.row("git.lines_removed", "peer", entity_id=ERIN).equals(
        target_value=10, p25=4, median=6, p75=8, min=2, max=10, n=5
    )
    r.row("git.lines_removed", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 10}
    )
    r.row("git.lines_removed", "breakdown", entity_id=ERIN, dimensions=CATEGORY_CODE).equals(
        value=10
    )

    r.row("git.prs_created", "period", entity_id=ERIN).equals(value=1)
    r.row("git.prs_created", "peer", entity_id=ERIN).equals(
        target_value=1, p25=1, median=1, p75=1, min=1, max=1, n=5
    )
    r.row("git.prs_created", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 1}
    )
    r.row("git.prs_created", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(value=1)

    r.row("git.prs_merged", "period", entity_id=ERIN).equals(value=1)
    r.row("git.prs_merged", "peer", entity_id=ERIN).equals(
        target_value=1, p25=1, median=1, p75=1, min=1, max=1, n=5
    )
    r.row("git.prs_merged", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 1}
    )
    r.row("git.prs_merged", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(value=1)

    r.row("git.merge_rate", "period", entity_id=ERIN).equals(value=100)
    r.row("git.merge_rate", "peer", entity_id=ERIN).equals(
        target_value=100, p25=100, median=100, p75=100, min=100, max=100, n=5
    )
    r.row("git.merge_rate", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 100}
    )
    r.row("git.merge_rate", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(value=100)

    r.row("git.commits_per_active_day", "period", entity_id=ERIN).equals(value=2)
    r.row("git.commits_per_active_day", "peer", entity_id=ERIN).equals(
        target_value=2, p25=1, median=1, p75=1, min=1, max=2, n=5
    )
    r.row("git.commits_per_active_day", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 2}
    )
    commits_per_repository = r.breakdown("git.commits_per_active_day")
    assert len(commits_per_repository) == 2
    assert (
        some(commits_per_repository, dimensions={"key": "repository"}, value=1.0)
        == commits_per_repository
    )

    r.row("git.commit_size", "period", entity_id=ERIN).equals(value=60)
    r.row("git.commit_size", "peer", entity_id=ERIN).equals(
        target_value=60, p25=24, median=36, p75=48, min=12, max=60, n=5
    )
    r.row("git.commit_size", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 60}
    )
    r.row("git.commit_size", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(value=60)
    commit_size_histogram = r.row("git.commit_size", "histogram", entity_id=ERIN)
    commit_size_histogram.contains(bins={"lo": 50, "count": 1})
    commit_size_histogram.contains(bins={"hi": 60, "count": 1})

    r.row("git.pr_size", "period", entity_id=ERIN).equals(value=50)
    r.row("git.pr_size", "peer", entity_id=ERIN).equals(
        target_value=50, p25=20, median=30, p75=40, min=10, max=50, n=5
    )
    r.row("git.pr_size", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 50}
    )
    r.row("git.pr_size", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(value=50)
    r.row("git.pr_size", "histogram", entity_id=ERIN).nonempty("bins")

    r.row("git.pr_cycle_time_h", "period", entity_id=ERIN).equals(value=5)
    r.row("git.pr_cycle_time_h", "peer", entity_id=ERIN).equals(
        target_value=5, p25=2, median=3, p75=4, min=1, max=5, n=5
    )
    r.row("git.pr_cycle_time_h", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 5}
    )
    r.row("git.pr_cycle_time_h", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(
        value=5
    )
    r.row("git.pr_cycle_time_h", "histogram", entity_id=ERIN).nonempty("bins")

    r.row("git.test_change_share", "period", entity_id=ERIN).equals(value=0)
    r.row("git.test_change_share", "peer", entity_id=ERIN).equals(
        target_value=0, p25=0, median=0, p75=0, min=0, max=0, n=5
    )
    r.row("git.test_change_share", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 0}
    )
    r.row("git.test_change_share", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(
        value=0
    )

    r.row("git.pr_abandonment_rate", "period", entity_id=ERIN).equals(value=0)
    r.row("git.pr_abandonment_rate", "peer", entity_id=ERIN).equals(
        target_value=0, p25=0, median=0, p75=0, min=0, max=0, n=5
    )
    r.row("git.pr_abandonment_rate", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 0}
    )
    r.row("git.pr_abandonment_rate", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(
        value=0
    )

    r.row("git.review_coverage", "period", entity_id=ERIN).equals(value=100)
    r.row("git.review_coverage", "peer", entity_id=ERIN).equals(
        target_value=100, p25=100, median=100, p75=100, min=100, max=100, n=5
    )
    r.row("git.review_coverage", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 100}
    )
    r.row("git.review_coverage", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(
        value=100
    )

    r.row("git.reviewers_per_pr", "period", entity_id=ERIN).equals(value=2)
    r.row("git.reviewers_per_pr", "peer", entity_id=ERIN).equals(
        target_value=2, p25=1, median=1, p75=1, min=1, max=2, n=5
    )
    r.row("git.reviewers_per_pr", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 2}
    )
    r.row("git.reviewers_per_pr", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(
        value=2
    )

    r.row("git.multi_reviewer_rate", "period", entity_id=ERIN).equals(value=100)
    r.row("git.multi_reviewer_rate", "peer", entity_id=ERIN).equals(
        target_value=100, p25=0, median=0, p75=0, min=0, max=100, n=5
    )
    r.row("git.multi_reviewer_rate", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 100}
    )
    r.row("git.multi_reviewer_rate", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(
        value=100
    )

    r.row("git.merges_without_approval_rate", "period", entity_id=ERIN).equals(value=0)
    r.row("git.merges_without_approval_rate", "peer", entity_id=ERIN).equals(
        target_value=0, p25=0, median=0, p75=0, min=0, max=0, n=5
    )
    r.row("git.merges_without_approval_rate", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 0}
    )
    r.row(
        "git.merges_without_approval_rate", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB
    ).equals(value=0)

    r.row("git.active_days", "period", entity_id=ERIN).equals(value=1)
    r.row("git.active_days", "peer", entity_id=ERIN).equals(
        target_value=1, p25=1, median=1, p75=1, min=1, max=1, n=5
    )
    r.row("git.active_days", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 1}
    )
    active_days_per_repository = r.breakdown("git.active_days")
    assert len(active_days_per_repository) == 2
    assert (
        some(active_days_per_repository, dimensions={"key": "repository"}, value=1.0)
        == active_days_per_repository
    )

    r.row("git.first_review_time_h", "period", entity_id=ERIN).equals(value=2.5)
    r.row("git.first_review_time_h", "peer", entity_id=ERIN).equals(
        target_value=2.5, p25=1, median=1.5, p75=2, min=0.5, max=2.5, n=5
    )
    r.row("git.first_review_time_h", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 2.5}
    )
    r.row("git.first_review_time_h", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(
        value=2.5
    )
    r.row("git.first_review_time_h", "histogram", entity_id=ERIN).nonempty("bins")

    r.row("git.review_wait_share", "period", entity_id=ERIN).equals(value=50)
    r.row("git.review_wait_share", "peer", entity_id=ERIN).equals(
        target_value=50, p25=50, median=50, p75=50, min=50, max=50, n=5
    )
    r.row("git.review_wait_share", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 50}
    )
    r.row("git.review_wait_share", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(
        value=50
    )
    r.row("git.review_wait_share", "histogram", entity_id=ERIN).nonempty("bins")

    r.row("git.review_to_merge_time_h", "period", entity_id=ERIN).equals(value=2.5)
    r.row("git.review_to_merge_time_h", "peer", entity_id=ERIN).equals(
        target_value=2.5, p25=1, median=1.5, p75=2, min=0.5, max=2.5, n=5
    )
    r.row("git.review_to_merge_time_h", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 2.5}
    )
    r.row(
        "git.review_to_merge_time_h", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB
    ).equals(value=2.5)
    r.row("git.review_to_merge_time_h", "histogram", entity_id=ERIN).nonempty("bins")

    r.row("git.approval_to_merge_time_h", "period", entity_id=ERIN).equals(value=2.5)
    r.row("git.approval_to_merge_time_h", "peer", entity_id=ERIN).equals(
        target_value=2.5, p25=1, median=1.5, p75=2, min=0.5, max=2.5, n=5
    )
    r.row("git.approval_to_merge_time_h", "timeseries", entity_id=ERIN).contains(
        points={"bucket_start": "2026-10-01", "value": 2.5}
    )
    r.row(
        "git.approval_to_merge_time_h", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB
    ).equals(value=2.5)
    r.row("git.approval_to_merge_time_h", "histogram", entity_id=ERIN).nonempty("bins")


def test_unified_git_metrics_empty_window(spec: SpecRun) -> None:
    """A window before any activity serves null for every count, rate, median and ratio."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "period"}]},
                    {"metric_key": "git.code_lines", "views": [{"view": "period"}]},
                    {"metric_key": "git.lines_added", "views": [{"view": "period"}]},
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                    {"metric_key": "git.prs_merged", "views": [{"view": "period"}]},
                    {"metric_key": "git.merge_rate", "views": [{"view": "period"}]},
                    {"metric_key": "git.commits_per_active_day", "views": [{"view": "period"}]},
                    {"metric_key": "git.commit_size", "views": [{"view": "period"}]},
                    {"metric_key": "git.pr_size", "views": [{"view": "period"}]},
                    {"metric_key": "git.pr_cycle_time_h", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("git.commits", "period", entity_id=ERIN).equals(value=None)
    r.row("git.code_lines", "period", entity_id=ERIN).equals(value=None)
    r.row("git.lines_added", "period", entity_id=ERIN).equals(value=None)
    r.row("git.prs_created", "period", entity_id=ERIN).equals(value=None)
    r.row("git.prs_merged", "period", entity_id=ERIN).equals(value=None)
    r.row("git.merge_rate", "period", entity_id=ERIN).equals(value=None)
    r.row("git.commits_per_active_day", "period", entity_id=ERIN).equals(value=None)
    r.row("git.commit_size", "period", entity_id=ERIN).equals(value=None)
    r.row("git.pr_size", "period", entity_id=ERIN).equals(value=None)
    r.row("git.pr_cycle_time_h", "period", entity_id=ERIN).equals(value=None)
