"""Bitbucket Cloud git metrics, served per person through the source breakdown.

Bronze: the connector's commits, file changes, pull requests, diffstat and activity
rows, driven through the bitbucket_cloud staging models into the shared git classes.
Line counts come from the file-change rows, so a commit with no file changes
contributes zero rather than dropping out; a merge commit is excluded from the commit
count and contributes no size; the even-count size median takes the upper middle.
"""

from __future__ import annotations

import pytest
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_commits_bitbucket"

ERIN = "erin@example.com"

SOURCE_BITBUCKET = {"key": "source", "value": "bitbucket_cloud"}


def test_bitbucket_git_metrics_resolve_through_the_source_breakdown(spec: SpecRun) -> None:
    """commit-a and commit-b count and the merge commit does not; lines come from the
    one file-change row; sizes {12, 0} give an upper-middle median of 12."""
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
                        "views": [{"view": "period"}, {"view": "breakdown", "dimensions": ["source"]}],
                    },
                    {"metric_key": "git.lines_added", "views": [{"view": "breakdown", "dimensions": ["source"]}]},
                    {"metric_key": "git.lines_removed", "views": [{"view": "breakdown", "dimensions": ["source"]}]},
                    {"metric_key": "git.commit_size", "views": [{"view": "breakdown", "dimensions": ["source"]}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commits", "breakdown", entity_id=ERIN, dimensions=SOURCE_BITBUCKET).equals(value=2)
    r.row("git.lines_added", "breakdown", entity_id=ERIN, dimensions=SOURCE_BITBUCKET).equals(value=10)
    r.row("git.lines_removed", "breakdown", entity_id=ERIN, dimensions=SOURCE_BITBUCKET).equals(value=2)
    r.row("git.commit_size", "breakdown", entity_id=ERIN, dimensions=SOURCE_BITBUCKET).equals(value=12)


def test_bitbucket_git_metrics_empty_window(spec: SpecRun) -> None:
    """A window with no commits serves a null commit count."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [{"metric_key": "git.commits", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.commits", "period", entity_id=ERIN).equals(value=None)
