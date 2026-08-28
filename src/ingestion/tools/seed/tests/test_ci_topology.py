from __future__ import annotations

import pytest

from insight_seed import profiles
from insight_seed.generators import ci_topology


@pytest.fixture
def base_roster() -> list[profiles.Person]:
    return profiles.build_roster("dev@company.nonpresent")


def test_every_vendor_is_represented_with_a_default_branch(
    base_roster: list[profiles.Person],
) -> None:
    grid = ci_topology.repo_grid(base_roster)
    vendors = {r.vendor for r in grid}
    assert vendors == {"github", "gitlab", "bitbucket"}
    assert all(r.default_branch for r in grid), "a repository with no default branch"


def test_one_github_repository_carries_no_pipelines(base_roster: list[profiles.Person]) -> None:
    """The 'absent, not zero' case: present in the repository class, no runs."""
    grid = ci_topology.repo_grid(base_roster)
    empty = [r for r in grid if r.vendor == "github" and not r.pipelines]
    assert len(empty) == 1
    assert empty[0].full_name == "acme/legacy-archive"


def test_no_non_github_repository_carries_pipelines(base_roster: list[profiles.Person]) -> None:
    grid = ci_topology.repo_grid(base_roster)
    assert all(not r.pipelines for r in grid if r.vendor != "github")


def test_run_weights_are_skewed_and_sum_to_one(base_roster: list[profiles.Person]) -> None:
    repos = ci_topology.ci_repos(ci_topology.repo_grid(base_roster))
    weights = sorted((r.weight for r in repos), reverse=True)
    assert abs(sum(weights) - 1.0) < 1e-9
    assert weights[0] > 2 * weights[-1], "volume is flat, not skewed"


def test_the_base_roster_gets_no_growth_repositories(base_roster: list[profiles.Person]) -> None:
    grid = ci_topology.repo_grid(base_roster)
    assert not [r for r in grid if r.full_name.startswith("acme/squad-")]


def test_a_grown_roster_gets_one_repository_per_extra_development_squad() -> None:
    roster = profiles.build_seeded_roster("dev@company.nonpresent", 200)
    grid = ci_topology.repo_grid(roster)
    squads = [r for r in grid if r.full_name.startswith("acme/squad-")]
    dev_leads = [p for p in roster if p.team == "development" and p.role == "lead"]
    assert len(squads) == len(dev_leads) - 1
