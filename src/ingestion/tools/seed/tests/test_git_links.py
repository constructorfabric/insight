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
from insight_seed.generators import base, git, git_history
from insight_seed.generators.git_history import build_history

_TENANT = "00000000-df51-5b42-9538-d2b56b7ee953"
_ANCHOR = _dt.date(2026, 8, 11)
_DAYS = 60

Table = tuple[list[str], list[tuple[Any, ...]]]
Rows = dict[str, Table]


@pytest.fixture(autouse=True)
def pinned_anchor(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(base, "_anchor_cache", _ANCHOR)


@pytest.fixture
def emitted(monkeypatch: pytest.MonkeyPatch) -> Rows:
    captured: Rows = {}

    def capture(
        client: Any, schema: str, table: str, cols: list[str], rows: list[tuple[Any, ...]]
    ) -> int:
        captured[f"{schema}.{table}"] = (list(cols), list(rows))
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


def _column(table: Table, name: str) -> list[Any]:
    cols, rows = table
    return [row[cols.index(name)] for row in rows]


def test_every_eligible_person_day_emits_at_least_one_commit(
    emitted: Rows, roster: list[profiles.Person]
) -> None:
    """A person-day with no commit is a hole in every git metric for that day."""
    history = build_history(roster, _DAYS)

    git.seed_commits(None, history, _TENANT)

    table = emitted["bronze_github.commits"]
    authored = set(
        zip(
            _column(table, "author_email"),
            [stamp[:10] for stamp in _column(table, "committed_date")],
            strict=True,
        )
    )
    expected = {
        (person.email, day.isoformat())
        for person in git_history.eligible(roster)
        for day in base.days_window(_DAYS)
    }
    assert authored == expected


def test_every_pr_gets_link_rows_and_links_point_at_emitted_commits(
    emitted: Rows, roster: list[profiles.Person]
) -> None:
    history = build_history(roster, _DAYS)

    git.seed_commits(None, history, _TENANT)
    git.seed_pull_requests(None, history, _TENANT)

    commit_shas = set(_column(emitted["bronze_github.commits"], "sha"))
    pr_numbers = set(_column(emitted["bronze_github.pull_requests"], "number"))
    links = emitted["bronze_github.pull_request_commits"]
    linked_prs = set(_column(links, "pull_number"))
    linked_shas = set(_column(links, "sha"))

    orphaned = pr_numbers - linked_prs
    assert not orphaned, (
        f"requests without link rows vanish from gold's pr_commit_emails join: {orphaned}"
    )
    dangling = linked_shas - commit_shas
    assert not dangling, f"link rows must point at commits the connector emitted: {dangling}"
