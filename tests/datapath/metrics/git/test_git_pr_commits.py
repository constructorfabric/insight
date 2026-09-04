"""Commits per pull request, served as the median per person over a window.

Bronze: pull requests, their per-request diff stats (which carry the author email)
and the request-to-commit link rows. Silver keys the links per (repo, request,
commit), so a re-synced duplicate link collapses instead of inflating the count.
Gold gives every merged request one value, its distinct linked commits, dated at
the merge day; a merged request with no link rows contributes no value, never a
zero, and its author stays out of the peer pool.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_pr_commits"

ALICE = "alice@example.com"
BOB = "bob@example.com"


def test_median_commits_per_merged_pull_request(spec: SpecRun) -> None:
    """alice's three merged requests carry 1, 2 and 4 commits, so the median is 2; bob's
    valueless request keeps him out of the pool, so n is 5, not 6."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.pr_commits",
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

    r.row("git.pr_commits", "period", entity_id=ALICE).equals(value=2)
    r.row("git.pr_commits", "peer", entity_id=ALICE).equals(
        target_value=2, p25=2, median=3, p75=5, min=1, max=6, n=5
    )
    r.row("git.pr_commits", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-01", "value": 2}
    )
    r.row("git.pr_commits", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-02", "value": 4}
    )
    r.row(
        "git.pr_commits",
        "breakdown",
        entity_id=ALICE,
        dimensions={"key": "source", "value": "github"},
    ).equals(value=2)
    r.row("git.pr_commits", "histogram", entity_id=ALICE).nonempty("bins")


def test_a_request_counts_on_its_merge_day_not_its_creation_day(spec: SpecRun) -> None:
    """The request created on Oct 01 and merged on Oct 02 lands in the Oct 02 window."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-10-02", "to": "2026-10-02"},
                "metrics": [{"metric_key": "git.pr_commits", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_commits", "period", entity_id=ALICE).equals(value=4)


def test_a_merged_request_without_linked_commit_rows_contributes_no_value(spec: SpecRun) -> None:
    """bob's merged request has no link rows, so his value is null, not zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [BOB]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [{"metric_key": "git.pr_commits", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_commits", "period", entity_id=BOB).equals(value=None)


def test_empty_window(spec: SpecRun) -> None:
    """A window with no merged requests serves null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "git.pr_commits", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.pr_commits", "period", entity_id=ALICE).equals(value=None)
