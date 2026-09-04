"""GitLab git metrics, served per person through the `source=gitlab` breakdown.

Bronze: the GitLab connector's projects, commits and commit file changes. Staging
drives them into the shared git classes. Commit size is additions plus deletions from
the commit's own stats; a merge commit (parent_count > 1) is excluded from the count
and the size pool; GitLab's diff API reports no object ids, so identical file rows on
two commits keep both commits' sizes and lines; per-file rows feed lines added/removed.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_commits_gitlab"

ERIN = "erin@example.com"

SOURCE_GITLAB = {"key": "source", "value": "gitlab"}


def test_gitlab_git_metrics_build_and_resolve_through_the_source_breakdown(spec: SpecRun) -> None:
    """Four commits without the merge; repeated content counts under both commits
    (24 added, 4 removed); sizes {12, 0, 8, 8} give median 8."""
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
                        "views": [{"view": "breakdown", "dimensions": ["source"]}],
                    },
                    {
                        "metric_key": "git.lines_added",
                        "views": [{"view": "breakdown", "dimensions": ["source"]}],
                    },
                    {
                        "metric_key": "git.lines_removed",
                        "views": [{"view": "breakdown", "dimensions": ["source"]}],
                    },
                    {
                        "metric_key": "git.commit_size",
                        "views": [{"view": "breakdown", "dimensions": ["source"]}],
                    },
                    {
                        "metric_key": "git.commits_per_active_day",
                        "views": [{"view": "breakdown", "dimensions": ["source"]}],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commits", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITLAB).equals(value=4)
    r.row("git.lines_added", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITLAB).equals(value=24)
    r.row("git.lines_removed", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITLAB).equals(
        value=4
    )
    r.row("git.commit_size", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITLAB).equals(value=8)
    r.row(
        "git.commits_per_active_day", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITLAB
    ).equals(value=4)


def test_gitlab_git_metrics_empty_window(spec: SpecRun) -> None:
    """A window before any commit serves an honest null for count and size."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "period"}]},
                    {"metric_key": "git.commit_size", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("git.commits", "period", entity_id=ERIN).equals(value=None)
    r.row("git.commit_size", "period", entity_id=ERIN).equals(value=None)
