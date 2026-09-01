"""The commit and pull-request skeleton every git-derived generator reads.

Owns the RNG draws for both. `git.py` shapes silver rows from these values and
`ci.py` anchors CI runs to them, so a run's head SHA is a hash git.py wrote.
"""

from __future__ import annotations

import datetime as _dt
import random
from collections.abc import Sequence
from dataclasses import dataclass

from ..profiles import TEAM_PROFILES, Person
from .base import (
    UTC,
    days_window,
    deterministic_int,
    deterministic_uuid,
    persona_multiplier,
    poisson,
    seeded_rng,
    weekday_multiplier,
)

# Hard per-person-per-day caps. Generation respects these by
# construction — they aren't validation rules, just upper bounds on
# the Poisson draws so the dataset stays plausible.
COMMITS_CAP = 20
PRS_CAP = 6


@dataclass(frozen=True)
class Commit:
    hash: str
    is_merge: bool
    lines_added: float
    lines_removed: float


@dataclass(frozen=True)
class PullRequest:
    pr_id: int
    created: _dt.datetime
    merged_on: _dt.datetime | None
    lines_added: float
    lines_removed: float


@dataclass(frozen=True)
class DayHistory:
    person: Person
    date: _dt.date
    commits: tuple[Commit, ...]
    prs: tuple[PullRequest, ...]


def eligible(roster: Sequence[Person]) -> list[Person]:
    """Persons whose team profile has any git weight."""
    return [p for p in roster if p.team and TEAM_PROFILES[p.team].weights.get("github", 0) > 0]


def daily_commit_count(rng: random.Random, mean: float) -> int:
    """The day's commit count, floored at one per person-day.

    The floor keeps every git bucket drillable: a zero-commit day renders its
    bucket undrillable in the dashboard's trailing windows.
    """
    return max(1, min(poisson(rng, mean), COMMITS_CAP))


def _commits_for(
    person: Person, day: _dt.date, persona: float, weight: float
) -> tuple[Commit, ...]:
    rng = seeded_rng(person.uuid, day, "git.commits")
    mean = 5 * persona * weight * weekday_multiplier(day)
    drawn = []
    for i in range(daily_commit_count(rng, mean)):
        sha = deterministic_uuid("git.commit", person.uuid, day.isoformat(), str(i))[:40]
        drawn.append(
            Commit(
                hash=sha.replace("-", ""),
                is_merge=rng.random() < 0.05,
                lines_added=float(rng.randint(2, 180)),
                lines_removed=float(rng.randint(0, 80)),
            )
        )
    return tuple(drawn)


def _prs_for(
    person: Person, day: _dt.date, persona: float, weight: float
) -> tuple[PullRequest, ...]:
    rng = seeded_rng(person.uuid, day, "git.prs")
    mean = 0.8 * persona * weight * weekday_multiplier(day)
    drawn = []
    for i in range(min(poisson(rng, mean), PRS_CAP)):
        pr_id = deterministic_int("git.pr", person.uuid, day.isoformat(), str(i))
        created = _dt.datetime.combine(
            day, _dt.time(9 + rng.randint(0, 8), rng.randint(0, 59), tzinfo=UTC)
        )
        # INVARIANT: the test is evaluated before the branch, so random() is
        # drawn before randint(1, 72). Reordering shifts every later draw.
        merged_in_h = rng.randint(1, 72) if rng.random() < 0.85 else None
        drawn.append(
            PullRequest(
                pr_id=pr_id,
                created=created,
                merged_on=(
                    created + _dt.timedelta(hours=merged_in_h) if merged_in_h is not None else None
                ),
                lines_added=float(rng.randint(20, 350)),
                lines_removed=float(rng.randint(0, 180)),
            )
        )
    return tuple(drawn)


def build_history(roster: Sequence[Person], days: int) -> list[DayHistory]:
    """One entry per (git-eligible person, day) in the window."""
    history: list[DayHistory] = []
    for person in eligible(roster):
        persona = persona_multiplier(person.uuid)
        weight = TEAM_PROFILES[person.team or ""].weights["github"]
        for day in days_window(days):
            history.append(
                DayHistory(
                    person=person,
                    date=day,
                    commits=_commits_for(person, day, persona, weight),
                    prs=_prs_for(person, day, persona, weight),
                )
            )
    return history
