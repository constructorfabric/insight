"""Git output split by branch scope, served per person over a window.

Three people whose per-metric numbers are identical, but below the peer percentile
disclosure threshold, each reach their default-branch total a different way: alice and
bob by the connector's flag, carol by a merged pull request into the default branch
while her flag still says otherwise. The merged-request heal is what makes the split
mean "did this work land"; her second request merges into a release branch and must
not promote its commit.
"""

from __future__ import annotations

import pytest
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_branch_scope"

ALICE = "alice@example.com"
CAROL = "carol@example.com"


def test_branch_scope_splits_partition_their_totals(spec: SpecRun) -> None:
    """One landed and one in-flight commit per person: each default/non-default pair
    sums to its total, and the peer pool of three stays below the percentile threshold."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.default_branch_commits",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.non_default_branch_commits",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.default_branch_code_lines",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.non_default_branch_code_lines",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["source"]},
                        ],
                    },
                    {
                        "metric_key": "git.default_branch_lines_added",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["category"]},
                        ],
                    },
                    {
                        "metric_key": "git.non_default_branch_lines_added",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["category"]},
                        ],
                    },
                    {
                        "metric_key": "git.default_branch_lines_removed",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["category"]},
                        ],
                    },
                    {
                        "metric_key": "git.non_default_branch_lines_removed",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["category"]},
                        ],
                    },
                    {
                        "metric_key": "git.default_branch_prs_created",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["destination_branch"]},
                        ],
                    },
                    {
                        "metric_key": "git.non_default_branch_prs_created",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["destination_branch"]},
                        ],
                    },
                    {
                        "metric_key": "git.default_branch_prs_merged",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["destination_branch"]},
                        ],
                    },
                    {
                        "metric_key": "git.non_default_branch_prs_merged",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                            {"view": "breakdown", "dimensions": ["destination_branch"]},
                        ],
                    },
                    {
                        "metric_key": "git.commits",
                        "views": [{"view": "period"}, {"view": "breakdown", "dimensions": ["branch_scope"]}],
                    },
                    {
                        "metric_key": "git.lines_added",
                        "views": [{"view": "period"}, {"view": "breakdown", "dimensions": ["branch_scope"]}],
                    },
                    {"metric_key": "git.commit_size", "views": [{"view": "breakdown", "dimensions": ["branch_scope"]}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.default_branch_commits", "period", entity_id=ALICE).equals(value=1)
    r.row("git.default_branch_commits", "peer", entity_id=ALICE).equals(
        target_value=1, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    r.row("git.default_branch_commits", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-01", "value": 1}
    )
    r.row(
        "git.default_branch_commits", "breakdown", entity_id=ALICE, dimensions={"key": "source", "value": "github"}
    ).equals(value=1)
    r.row("git.non_default_branch_commits", "period", entity_id=ALICE).equals(value=1)
    r.row("git.non_default_branch_commits", "peer", entity_id=ALICE).equals(
        target_value=1, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    r.row("git.non_default_branch_commits", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-01", "value": 1}
    )
    r.row(
        "git.non_default_branch_commits", "breakdown", entity_id=ALICE, dimensions={"key": "source", "value": "github"}
    ).equals(value=1)
    r.row("git.commits", "period", entity_id=ALICE).equals(value=2)

    r.row("git.default_branch_code_lines", "period", entity_id=ALICE).equals(value=10)
    r.row("git.default_branch_code_lines", "peer", entity_id=ALICE).equals(
        target_value=10, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    r.row("git.default_branch_code_lines", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-01", "value": 10}
    )
    r.row(
        "git.default_branch_code_lines", "breakdown", entity_id=ALICE, dimensions={"key": "source", "value": "github"}
    ).equals(value=10)
    r.row("git.non_default_branch_code_lines", "period", entity_id=ALICE).equals(value=20)
    r.row("git.non_default_branch_code_lines", "peer", entity_id=ALICE).equals(
        target_value=20, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    r.row("git.non_default_branch_code_lines", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-01", "value": 20}
    )
    r.row(
        "git.non_default_branch_code_lines",
        "breakdown",
        entity_id=ALICE,
        dimensions={"key": "source", "value": "github"},
    ).equals(value=20)

    r.row("git.default_branch_lines_added", "period", entity_id=ALICE).equals(value=10)
    r.row("git.default_branch_lines_added", "peer", entity_id=ALICE).equals(
        target_value=10, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    r.row("git.default_branch_lines_added", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-01", "value": 10}
    )
    r.row(
        "git.default_branch_lines_added", "breakdown", entity_id=ALICE, dimensions={"key": "category", "value": "code"}
    ).equals(value=10)
    r.row("git.non_default_branch_lines_added", "period", entity_id=ALICE).equals(value=20)
    r.row("git.non_default_branch_lines_added", "peer", entity_id=ALICE).equals(
        target_value=20, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    r.row("git.non_default_branch_lines_added", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-01", "value": 20}
    )
    r.row(
        "git.non_default_branch_lines_added",
        "breakdown",
        entity_id=ALICE,
        dimensions={"key": "category", "value": "code"},
    ).equals(value=20)
    r.row("git.lines_added", "period", entity_id=ALICE).equals(value=30)

    r.row("git.default_branch_lines_removed", "period", entity_id=ALICE).equals(value=1)
    r.row("git.default_branch_lines_removed", "peer", entity_id=ALICE).equals(
        target_value=1, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    r.row("git.default_branch_lines_removed", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-01", "value": 1}
    )
    r.row(
        "git.default_branch_lines_removed",
        "breakdown",
        entity_id=ALICE,
        dimensions={"key": "category", "value": "code"},
    ).equals(value=1)
    r.row("git.non_default_branch_lines_removed", "period", entity_id=ALICE).equals(value=2)
    r.row("git.non_default_branch_lines_removed", "peer", entity_id=ALICE).equals(
        target_value=2, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    r.row("git.non_default_branch_lines_removed", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-01", "value": 2}
    )
    r.row(
        "git.non_default_branch_lines_removed",
        "breakdown",
        entity_id=ALICE,
        dimensions={"key": "category", "value": "code"},
    ).equals(value=2)

    r.row("git.default_branch_prs_created", "period", entity_id=ALICE).equals(value=1)
    r.row("git.default_branch_prs_created", "peer", entity_id=ALICE).equals(
        target_value=1, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    r.row("git.default_branch_prs_created", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-01", "value": 1}
    )
    r.row(
        "git.default_branch_prs_created",
        "breakdown",
        entity_id=ALICE,
        dimensions={"key": "destination_branch", "value": "main"},
    ).equals(value=1)
    r.row("git.non_default_branch_prs_created", "period", entity_id=ALICE).equals(value=1)
    r.row("git.non_default_branch_prs_created", "peer", entity_id=ALICE).equals(
        target_value=1, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    r.row("git.non_default_branch_prs_created", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-01", "value": 1}
    )
    r.row(
        "git.non_default_branch_prs_created",
        "breakdown",
        entity_id=ALICE,
        dimensions={"key": "destination_branch", "value": "release/1.x"},
    ).equals(value=1)

    r.row("git.default_branch_prs_merged", "period", entity_id=ALICE).equals(value=1)
    r.row("git.default_branch_prs_merged", "peer", entity_id=ALICE).equals(
        target_value=1, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    r.row("git.default_branch_prs_merged", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-01", "value": 1}
    )
    r.row(
        "git.default_branch_prs_merged",
        "breakdown",
        entity_id=ALICE,
        dimensions={"key": "destination_branch", "value": "main"},
    ).equals(value=1)
    r.row("git.non_default_branch_prs_merged", "period", entity_id=ALICE).equals(value=1)
    r.row("git.non_default_branch_prs_merged", "peer", entity_id=ALICE).equals(
        target_value=1, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    r.row("git.non_default_branch_prs_merged", "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-10-01", "value": 1}
    )
    r.row(
        "git.non_default_branch_prs_merged",
        "breakdown",
        entity_id=ALICE,
        dimensions={"key": "destination_branch", "value": "release/1.x"},
    ).equals(value=1)

    r.row("git.commits", "breakdown", entity_id=ALICE, dimensions={"key": "branch_scope", "value": "default"}).equals(
        value=1
    )
    r.row(
        "git.commits", "breakdown", entity_id=ALICE, dimensions={"key": "branch_scope", "value": "non_default"}
    ).equals(value=1)
    r.row(
        "git.lines_added", "breakdown", entity_id=ALICE, dimensions={"key": "branch_scope", "value": "default"}
    ).equals(value=10)
    r.row(
        "git.lines_added", "breakdown", entity_id=ALICE, dimensions={"key": "branch_scope", "value": "non_default"}
    ).equals(value=20)

    r.row(
        "git.commit_size", "breakdown", entity_id=ALICE, dimensions={"key": "branch_scope", "value": "default"}
    ).equals(value=11)
    r.row(
        "git.commit_size", "breakdown", entity_id=ALICE, dimensions={"key": "branch_scope", "value": "non_default"}
    ).equals(value=22)


def test_a_merged_request_into_the_default_branch_promotes_its_commit(spec: SpecRun) -> None:
    """Both of carol's commits are flagged outside the default branch; the one her merged
    request into main carries counts as landed, the one her release request carries does not."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.default_branch_commits", "views": [{"view": "period"}]},
                    {"metric_key": "git.non_default_branch_commits", "views": [{"view": "period"}]},
                    {"metric_key": "git.default_branch_lines_added", "views": [{"view": "period"}]},
                    {"metric_key": "git.non_default_branch_lines_added", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200
    r.row("git.default_branch_commits", "period", entity_id=CAROL).equals(value=1)
    r.row("git.non_default_branch_commits", "period", entity_id=CAROL).equals(value=1)
    r.row("git.default_branch_lines_added", "period", entity_id=CAROL).equals(value=10)
    r.row("git.non_default_branch_lines_added", "period", entity_id=CAROL).equals(value=20)
