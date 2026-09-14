"""Requests opened against the repository's own default branch.

The destination is compared against THAT repository's declared default, and the scope
picks one of the pair, so default plus other always equals the total. Three wrong
readings pass the two specs that already assert this comparison, because both use one
repository whose default is `main` with every request carrying a destination: a
comparison hardcoded to `main`, one letting an empty destination match an unknown
default, and one that stops selecting branches on the default flag at all.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_default_branch_prs_created"

ERIN = "erin@example.com"
WINDOW = {"from": "2026-10-01", "to": "2026-10-02"}


def test_a_destination_counts_only_against_its_own_repositorys_default(spec: SpecRun) -> None:
    """91 into `main` and 93 into `master` count; the other four do not.

    Letting an empty destination match an unknown default reads 3. The merged pair
    rides the same requests, dated by the merge rather than by the opening.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": WINDOW,
                "metrics": [
                    {"metric_key": "git.default_branch_prs_created", "views": [{"view": "period"}]},
                    {
                        "metric_key": "git.non_default_branch_prs_created",
                        "views": [{"view": "period"}],
                    },
                    {"metric_key": "git.prs_created", "views": [{"view": "period"}]},
                    {"metric_key": "git.default_branch_prs_merged", "views": [{"view": "period"}]},
                    {
                        "metric_key": "git.non_default_branch_prs_merged",
                        "views": [{"view": "period"}],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.default_branch_prs_created", "period", entity_id=ERIN).equals(value=2)
    r.row("git.non_default_branch_prs_created", "period", entity_id=ERIN).equals(value=4)
    # The pair partitions the total rather than merely bounding it.
    r.row("git.prs_created", "period", entity_id=ERIN).equals(value=6)

    r.row("git.default_branch_prs_merged", "period", entity_id=ERIN).equals(value=1)
    r.row("git.non_default_branch_prs_merged", "period", entity_id=ERIN).equals(value=1)


def test_two_repositories_can_disagree_about_the_default_branch_name(spec: SpecRun) -> None:
    """One each. A comparison hardcoded to `main` keeps the total at 2 but reads `main`
    twice and `master` not at all, which only this split can see."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": WINDOW,
                "metrics": [
                    {
                        "metric_key": "git.default_branch_prs_created",
                        "views": [{"view": "breakdown", "dimensions": ["destination_branch"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    for destination in ("main", "master"):
        r.row(
            "git.default_branch_prs_created",
            "breakdown",
            entity_id=ERIN,
            dimensions={"key": "destination_branch", "value": destination},
        ).equals(value=1)

    destinations = r.breakdown("git.default_branch_prs_created")
    assert len(destinations) == 2, f"a third destination appeared: {destinations!r}"


def test_a_repository_that_reported_no_branches_reaches_one_scope_only(spec: SpecRun) -> None:
    """Both halves are needed: without the positive rule on the other scope, the absence
    below is satisfied by a repository that never reached the breakdown at all."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": WINDOW,
                "metrics": [
                    {
                        "metric_key": "git.default_branch_prs_created",
                        "views": [{"view": "breakdown", "dimensions": ["repository"]}],
                    },
                    {
                        "metric_key": "git.non_default_branch_prs_created",
                        "views": [{"view": "breakdown", "dimensions": ["repository"]}],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    for repository in ("git-test:acme/mainline", "git-test:acme/legacy"):
        r.row(
            "git.default_branch_prs_created",
            "breakdown",
            entity_id=ERIN,
            dimensions={"key": "repository", "value": repository},
        ).equals(value=1)

    r.row(
        "git.non_default_branch_prs_created",
        "breakdown",
        entity_id=ERIN,
        dimensions={"key": "repository", "value": "git-test:acme/nodefault"},
    ).equals(value=2)

    # The opposite scope needs its own cardinality check: the rows it holds for
    # the other two repositories are never selected, so nothing else counts them
    # and a leaked dimension value would pass unseen.
    missed = r.breakdown("git.non_default_branch_prs_created")
    assert len(missed) == 3, f"the opposite scope gained or lost a repository: {missed!r}"

    landed = r.breakdown("git.default_branch_prs_created")
    assert len(landed) == 2, f"a third repository appeared: {landed!r}"
    assert not any(
        dimension.get("value") == "git-test:acme/nodefault"
        for entry in landed
        for dimension in entry.get("dimensions", [])
    ), f"a repository that reported no branches reached this scope: {landed!r}"


def test_an_empty_window_is_null_not_zero(spec: SpecRun) -> None:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {"metric_key": "git.default_branch_prs_created", "views": [{"view": "period"}]}
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.default_branch_prs_created", "period", entity_id=ERIN).equals(value=None)
