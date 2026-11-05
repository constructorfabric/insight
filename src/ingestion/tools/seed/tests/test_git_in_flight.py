"""Which seeded commits sit outside the default branch."""

from __future__ import annotations

import datetime as _dt

import pytest

from insight_seed.generators import git
from insight_seed.generators.git_history import Commit, DayHistory, PullRequest
from insight_seed.profiles import Person

_PERSON = Person(uuid="p", email="p@example.com", team="dev", role="ic", parent_uuid=None)
_DAY = _dt.date(2026, 6, 1)
_AT = _dt.datetime(2026, 6, 1, 9, tzinfo=_dt.UTC)


def _day(commit_count: int, merged: list[bool]) -> DayHistory:
    commits = tuple(
        Commit(hash=f"{i:040x}", is_merge=False, lines_added=1, lines_removed=0)
        for i in range(commit_count)
    )
    prs = tuple(
        PullRequest(
            pr_id=i, created=_AT, merged_on=_AT if done else None, lines_added=1, lines_removed=0
        )
        for i, done in enumerate(merged)
    )
    return DayHistory(person=_PERSON, date=_DAY, commits=commits, prs=prs)


def _sha(index: int) -> str:
    return git._sha(f"{index:040x}")


@pytest.mark.parametrize(
    ("case", "day", "expected"),
    [
        ("no requests: every commit was pushed to the default branch", _day(3, []), set()),
        ("a merged request lands its commits", _day(2, [True]), set()),
        ("an unmerged request keeps its commits off", _day(2, [False]), {_sha(0), _sha(1)}),
        ("only the unmerged request's share stays off", _day(4, [True, False]), {_sha(1), _sha(3)}),
        ("a commit shared with a merged request has landed", _day(1, [False, True]), set()),
    ],
)
def test_in_flight_commits_are_those_only_unmerged_requests_link(
    case: str, day: DayHistory, expected: set[str]
) -> None:
    assert git.in_flight_shas([day]) == expected, f"case: {case}"
