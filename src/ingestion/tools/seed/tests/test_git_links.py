"""Count parity between the git commit generator and the PR-link generator.

`seed_class_git_pull_requests_commits` re-derives each day's commit list from
the same RNG stream `seed_class_git_commits` draws from, so both must compute
the day's commit count with one formula. Locked regression: the commit
generator floors the count at one per person-day, and a links generator still
using the raw Poisson draw skips exactly those floored days — a PR created on
one has no link rows, gold's `pr_commit_emails` CTE misses it, and the PR's
author dimensions NULL out.

The anchor is pinned because the window contents decide whether the regression
is reachable: the fixture window must contain at least one day with a PR draw
above zero and a commit draw of zero.

Run against the installed package (see the README's develop section):

    uv run --extra dev pytest tests
"""

from __future__ import annotations

import datetime as _dt
from typing import Any

import pytest

from insight_seed import profiles
from insight_seed.generators import base, git

_TENANT = "00000000-df51-5b42-9538-d2b56b7ee953"
_ANCHOR = _dt.date(2026, 8, 11)
_DAYS = 60

Rows = dict[str, list[tuple[Any, ...]]]


@pytest.fixture(autouse=True)
def pinned_anchor(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(base, "_anchor_cache", _ANCHOR)


@pytest.fixture
def emitted(monkeypatch: pytest.MonkeyPatch) -> Rows:
    captured: Rows = {}

    def capture(
        client: Any, schema: str, table: str, cols: list[str], rows: list[tuple[Any, ...]]
    ) -> int:
        captured[f"{schema}.{table}"] = list(rows)
        return len(rows)

    monkeypatch.setattr(git, "truncate", lambda client, schema, table: None)
    monkeypatch.setattr(git, "bulk_insert", capture)
    return captured


@pytest.fixture
def roster() -> list[profiles.Person]:
    return profiles.build_roster("dev@company.nonpresent")


def test_the_fixture_window_reaches_the_zero_draw_case(roster: list[profiles.Person]) -> None:
    zero_commit_pr_days = 0
    for person in git._eligible(roster):
        persona = base.persona_multiplier(person.uuid)
        weight = profiles.TEAM_PROFILES[person.team or ""].weights["github"]
        for day in base.days_window(_DAYS):
            commit_rng = base.seeded_rng(person.uuid, day, "git.commits")
            commit_mean = 5 * persona * weight * base.weekday_multiplier(day)
            commit_draw = min(base.poisson(commit_rng, commit_mean), git.COMMITS_CAP)

            pr_rng = base.seeded_rng(person.uuid, day, "git.prs")
            pr_mean = 0.8 * persona * weight * base.weekday_multiplier(day)
            if commit_draw == 0 and min(base.poisson(pr_rng, pr_mean), git.PRS_CAP) > 0:
                zero_commit_pr_days += 1

    assert zero_commit_pr_days > 0, (
        "no day in the pinned window draws a PR with a zero commit draw, "
        "so the parity tests below cannot catch a count divergence"
    )


def test_every_eligible_person_day_emits_at_least_one_commit(
    emitted: Rows, roster: list[profiles.Person]
) -> None:
    git.seed_class_git_commits(None, roster, _TENANT, _DAYS)

    person_days = {(row[6], row[7]) for row in emitted["silver.class_git_commits"]}
    expected = {
        (person.email, day) for person in git._eligible(roster) for day in base.days_window(_DAYS)
    }
    assert person_days == expected


def test_every_pr_gets_link_rows_and_links_point_at_emitted_commits(
    emitted: Rows, roster: list[profiles.Person]
) -> None:
    git.seed_class_git_commits(None, roster, _TENANT, _DAYS)
    git.seed_class_git_pull_requests(None, roster, _TENANT, _DAYS)
    git.seed_class_git_pull_requests_commits(None, roster, _TENANT, _DAYS)

    commit_hashes = {row[1] for row in emitted["silver.class_git_commits"]}
    pr_ids = {row[1] for row in emitted["silver.class_git_pull_requests"]}
    links = emitted["silver.class_git_pull_requests_commits"]
    linked_prs = {row[4] for row in links}
    linked_hashes = {row[5] for row in links}

    orphaned_prs = pr_ids - linked_prs
    assert not orphaned_prs, (
        f"PRs without link rows vanish from gold's pr_commit_emails join: {orphaned_prs}"
    )

    dangling_links = linked_hashes - commit_hashes
    assert not dangling_links, (
        f"link rows must point at commits class_git_commits emitted: {dangling_links}"
    )
