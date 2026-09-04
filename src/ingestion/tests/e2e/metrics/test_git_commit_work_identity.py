"""Commit metrics count an authored change once, however many commits re-apply it.

A merged pull request's result commit (a squash, or the last rebased copy) contributes
neither a commit nor lines, its own recombined file rows included. A commit carrying a
patch id an earlier commit already carries is the same authored change, kept by the
earliest commit, and patch identity folds within a repository only. When one hash sits
in two connected repositories, its file rows attach to the commit row that survives the
collapse. Commit size reads the same commit set as the commit and line figures.
"""

from __future__ import annotations

import pytest
from lib.metric_expect import MetricResponse, Row, some
from lib.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_commit_work_identity"

ERIN = "erin@example.com"

SQUASH_MERGE = "git-test:constructor/squash-merge"
FAST_FORWARD = "git-test:constructor/fast-forward"
PATCH_COPY = "git-test:constructor/patch-copy"
UNSEEN_BRANCH = "git-test:constructor/unseen-branch"
PARTIAL_BRANCH = "git-test:constructor/partial-branch"
INDEP_A = "git-test:constructor/indep-a"
INDEP_B = "git-test:constructor/indep-b"
XR_FORK = "git-test:constructor/xr-fork"
XR_UPSTREAM = "git-test:constructor/xr-upstream"


def by_repository(r: MetricResponse, metric: str, repository: str) -> Row:
    return r.row(metric, "breakdown", entity_id=ERIN, dimensions={"key": "repository", "value": repository})


def test_squash_result_contributes_neither_commit_nor_recombined_lines(spec: SpecRun) -> None:
    """The three branch originals count once each; the squash result adds no commit, none
    of its A-to-C lines, and nothing to the size pool {10, 10, 6}."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                    {"metric_key": "git.lines_added", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                    {"metric_key": "git.lines_removed", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                    {"metric_key": "git.commit_size", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                ],
            },
        }
    )
    assert r.status == 200

    by_repository(r, "git.commits", SQUASH_MERGE).equals(value=3)
    by_repository(r, "git.lines_added", SQUASH_MERGE).equals(value=23)
    by_repository(r, "git.lines_removed", SQUASH_MERGE).equals(value=3)
    by_repository(r, "git.commit_size", SQUASH_MERGE).equals(value=10)


def test_fast_forward_merge_promotes_an_original_which_stays_authored(spec: SpecRun) -> None:
    """The merged request's result hash is a branch commit itself, so it keeps its work."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                    {"metric_key": "git.lines_added", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                ],
            },
        }
    )
    assert r.status == 200

    by_repository(r, "git.commits", FAST_FORWARD).equals(value=1)
    by_repository(r, "git.lines_added", FAST_FORWARD).equals(value=7)


def test_one_patch_id_is_one_authored_change_kept_by_the_earliest_commit(spec: SpecRun) -> None:
    """October keeps the patch: one commit, its 6 added lines, and a size of 6 plus 1."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                    {"metric_key": "git.lines_added", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                    {"metric_key": "git.commit_size", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                ],
            },
        }
    )
    assert r.status == 200

    by_repository(r, "git.commits", PATCH_COPY).equals(value=1)
    by_repository(r, "git.lines_added", PATCH_COPY).equals(value=6)
    by_repository(r, "git.commit_size", PATCH_COPY).equals(value=7)


def test_the_later_carrier_of_an_already_authored_patch_reports_nothing(spec: SpecRun) -> None:
    """The November copy carries the October patch: no commit row, no lines and no size
    for the repository in November."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-11-01", "to": "2026-11-30"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                    {"metric_key": "git.lines_added", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                    {"metric_key": "git.commit_size", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                ],
            },
        }
    )
    assert r.status == 200

    assert some(r.breakdown("git.commits"), dimensions={"key": "repository", "value": PATCH_COPY}) == []
    assert some(r.breakdown("git.lines_added"), dimensions={"key": "repository", "value": PATCH_COPY}) == []
    assert some(r.breakdown("git.commit_size"), dimensions={"key": "repository", "value": PATCH_COPY}) == []


def test_merge_result_with_never_collected_branch_commits_stays_counted(spec: SpecRun) -> None:
    """The result commit is the only record of the work, so it keeps its commit and lines."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                    {"metric_key": "git.lines_added", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                ],
            },
        }
    )
    assert r.status == 200

    by_repository(r, "git.commits", UNSEEN_BRANCH).equals(value=1)
    by_repository(r, "git.lines_added", UNSEEN_BRANCH).equals(value=9)


def test_file_rows_under_the_repository_that_lost_the_collapse_still_count(spec: SpecRun) -> None:
    """One hash, one commit under the surviving repository; the lines recorded only under
    the losing repository attach to it, and the size stays the commit's own 4."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                    {"metric_key": "git.lines_added", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                    {"metric_key": "git.commit_size", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                ],
            },
        }
    )
    assert r.status == 200

    by_repository(r, "git.commits", XR_FORK).equals(value=1)
    assert some(r.breakdown("git.commits"), dimensions={"key": "repository", "value": XR_UPSTREAM}) == []
    by_repository(r, "git.lines_added", XR_FORK).equals(value=4)
    by_repository(r, "git.commit_size", XR_FORK).equals(value=4)


def test_merge_result_with_partly_collected_branch_commits_stays_counted(spec: SpecRun) -> None:
    """One of two linked commits was collected, so both commits count; the overlapping
    blob folds once (3 + 2 lines) and the sizes {3, 2} read 3."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                    {"metric_key": "git.lines_added", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                    {"metric_key": "git.commit_size", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                ],
            },
        }
    )
    assert r.status == 200

    by_repository(r, "git.commits", PARTIAL_BRANCH).equals(value=2)
    by_repository(r, "git.lines_added", PARTIAL_BRANCH).equals(value=5)
    by_repository(r, "git.commit_size", PARTIAL_BRANCH).equals(value=3)


def test_same_patch_id_in_unrelated_repositories_is_two_authored_changes(spec: SpecRun) -> None:
    """Patch identity folds within a repository only: neither independent change loses."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                    {"metric_key": "git.lines_added", "views": [{"view": "breakdown", "dimensions": ["repository"]}]},
                ],
            },
        }
    )
    assert r.status == 200

    by_repository(r, "git.commits", INDEP_A).equals(value=1)
    by_repository(r, "git.commits", INDEP_B).equals(value=1)
    by_repository(r, "git.lines_added", INDEP_A).equals(value=2)
    by_repository(r, "git.lines_added", INDEP_B).equals(value=2)
