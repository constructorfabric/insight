"""Authored commits that reached the repository's default branch.

A commit counts unless it is derived, and its scope is the connector's flag OR membership
of the default-branch set, which a merged request into the default branch confers on every
commit it links. The derived rule has owners elsewhere, but none of their fixtures carries
a flag or a branches row, so none can say how the rule and the scope compose: dropping a
squash removes a commit that was on the default branch, and the count survives that only
if the links it stood on were promoted in its place.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_default_branch_commits"

ERIN = "erin@example.com"


def test_every_collected_commit_landed_so_the_other_scope_holds_nobody(spec: SpecRun) -> None:
    """Each branch commit is promoted by its own request and each squash carries the
    connector's flag, so the non-default scope has no value at all."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.default_branch_commits", "views": [{"view": "period"}]},
                    {"metric_key": "git.non_default_branch_commits", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.default_branch_commits", "period", entity_id=ERIN).equals(value=6)
    r.row("git.non_default_branch_commits", "period", entity_id=ERIN).equals(value=None)


def test_a_squash_is_dropped_only_when_every_commit_it_stands_for_was_collected(
    spec: SpecRun,
) -> None:
    """Four repositories differing only in what the merged request reports.

    `full`: the squash drops and its two originals take its place — a rule that never
    fired reads 3, a heal that stopped promoting reads 0. `partial`: one promoted original
    plus the squash still standing for the two missing ones, where both dropping
    regardless of coverage and losing the heal read 1, for opposite reasons. `none`: the
    squash carries its own flag, so nothing here depends on the heal. `nolist`: an empty
    commit list must not read as complete coverage.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.default_branch_commits",
                        "views": [{"view": "breakdown", "dimensions": ["repository"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    for repository, commits in (
        ("git-test:acme/full", 2),
        ("git-test:acme/partial", 2),
        ("git-test:acme/none", 1),
        ("git-test:acme/nolist", 1),
    ):
        r.row(
            "git.default_branch_commits",
            "breakdown",
            entity_id=ERIN,
            dimensions={"key": "repository", "value": repository},
        ).equals(value=commits)

    repositories = r.breakdown("git.default_branch_commits")
    assert len(repositories) == 4, f"a fifth repository row appeared: {repositories!r}"


def test_an_empty_window_is_null_not_zero(spec: SpecRun) -> None:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {"metric_key": "git.default_branch_commits", "views": [{"view": "period"}]}
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.default_branch_commits", "period", entity_id=ERIN).equals(value=None)
