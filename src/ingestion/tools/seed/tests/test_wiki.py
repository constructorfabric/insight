"""The wiki generator emits revisions and comments that count back to its draws.

`outline__wiki_activity` collapses revisions into edit sessions and reads a
document's first revision as its creation; `outline__wiki_engagement` buckets a
comment as exactly one of footer / inline / reply. These apply both rule sets in
Python — the cheap half of the contract, the expensive half needing a warehouse.
"""

from __future__ import annotations

import datetime as _dt
from collections import defaultdict
from collections.abc import Iterator
from itertools import pairwise

import pytest

from insight_seed.generators import wiki
from insight_seed.generators.base import days_window
from insight_seed.profiles import TEAM_PROFILES, build_roster

DEV_EMAIL = "dev@company.nonpresent"
DAYS = 14
ANCHOR = _dt.date(2026, 10, 1)
SESSION_GAP = _dt.timedelta(seconds=1800)


@pytest.fixture(autouse=True, scope="module")
def _pinned_anchor() -> Iterator[None]:
    """Every draw below walks a window ending at the anchor, so an ambient one
    would make these tests answer differently each day."""
    from insight_seed.generators import base

    previous = base._anchor_cache
    base._anchor_cache = ANCHOR
    yield
    base._anchor_cache = previous


@pytest.fixture(scope="module")
def plan() -> tuple[list, list, list]:
    roster = build_roster(DEV_EMAIL)
    return roster, wiki._backlog_pages(roster, DAYS), wiki._plan_pages(roster, DAYS)


def test_the_backlog_covers_a_days_widest_edit(plan: tuple) -> None:
    """Edits land on documents. An author whose backlog is thinner than
    EDITS_CAP would run out on the first days and under-report pages_edited."""
    _, backlog, _ = plan
    per_author: dict[str, int] = defaultdict(int)
    for page in backlog:
        per_author[page.author.uuid] += 1

    assert per_author, "no backlog was planned"
    assert min(per_author.values()) >= wiki.EDITS_CAP, (
        f"an author starts with {min(per_author.values())} pages but a day may edit "
        f"{wiki.EDITS_CAP}"
    )


def test_the_backlog_predates_the_window(plan: tuple) -> None:
    """A backlog page created inside the window would count as created there."""
    _, backlog, _ = plan
    opens = days_window(DAYS)[0]

    assert all(page.created_at.date() < opens for page in backlog), (
        "a backlog page is dated inside the seeded window"
    )


def test_the_page_plan_is_unchanged(plan: tuple) -> None:
    """The planner's draws decide the page set; this pins what they yield."""
    _, _, planned = plan

    assert len(planned) == 48, (
        f"the planner produced {len(planned)} pages. If deliberate, every stand's "
        f"wiki page set changed with it."
    )


def test_a_comment_falls_in_exactly_one_bucket() -> None:
    """footer / inline / reply are disjoint and exhaustive in the staging model,
    so a comment counted twice would make the buckets exceed the total."""
    roster = build_roster(DEV_EMAIL)
    planned = wiki._plan_pages(roster, DAYS)
    last_day = days_window(DAYS)[-1]

    from insight_seed.generators.base import poisson, seeded_rng, weekday_multiplier

    for page in planned:
        published_on = page.created_at.date()
        rng = seeded_rng(page.author.uuid, published_on, f"wiki.comments.{page.page_id}")
        for offset in range(wiki.COMMENT_WINDOW_DAYS):
            day = published_on + _dt.timedelta(days=offset)
            if day > last_day:
                break
            total = min(poisson(rng, 0.25 * weekday_multiplier(day)), wiki.COMMENTS_CAP)
            if total == 0:
                continue
            replies = int(total * rng.uniform(0.0, 0.5))
            footer = rng.randint(0, total - replies)
            voices = max(1, int(total * rng.uniform(0.5, 1.0)))
            inline = total - replies - footer

            assert replies + footer + inline == total, (
                f"buckets do not sum for {page.page_id} on {day}"
            )
            assert 1 <= voices <= total, (
                f"unique commenters out of range for {page.page_id} on {day}"
            )


def test_the_revisions_count_back_to_the_drawn_activity(plan: tuple) -> None:
    """Sessions and ordinals, as `outline__wiki_activity` computes them."""
    from insight_seed.generators.base import (
        persona_multiplier,
        poisson,
        seeded_rng,
        weekday_multiplier,
    )

    roster, backlog, planned = plan
    by_author_day: dict[tuple[str, _dt.date], list] = defaultdict(list)
    for page in planned:
        by_author_day[(page.author.uuid, page.created_at.date())].append(page)
    backlog_by_author: dict[str, list] = defaultdict(list)
    for page in backlog:
        backlog_by_author[page.author.uuid].append(page)

    checked = 0
    for person in wiki._wiki_authors(roster):
        persona = persona_multiplier(person.uuid)
        weight = TEAM_PROFILES[person.team or ""].weights[wiki.SOURCE]
        owned = sorted(backlog_by_author[person.uuid], key=lambda page: page.page_id)
        for day_index, day in enumerate(days_window(DAYS)):
            rng = seeded_rng(person.uuid, day, "wiki.activity")
            mean = 1.2 * persona * weight * weekday_multiplier(day)
            new_pages = by_author_day.get((person.uuid, day), [])
            created = len(new_pages)
            edits = max(min(poisson(rng, mean), wiki.EDITS_CAP), created)
            if edits == 0:
                owned.extend(new_pages)
                continue
            edited = max(created, 1, int(edits * rng.uniform(0.4, 0.9)))

            older = [page for page in owned if page.created_at.date() < day]
            offset = day_index % max(len(older), 1)
            rotated = older[offset:] + older[:offset]
            targets = [*new_pages, *rotated][:edited]

            # Sessions, the way the model counts them: a new one whenever the
            # gap to the previous revision of that page exceeds 30 minutes.
            sessions_total = 0
            for position, page in enumerate(targets):
                sessions = 1 + (edits - edited + len(targets) - 1 - position) // len(targets)
                times = wiki._revision_times(day, sessions)
                gaps = [b - a for a, b in pairwise(times)]
                assert all(gap > SESSION_GAP for gap in gaps), (
                    f"revisions of {page.page_id} on {day} collapse into one session"
                )
                sessions_total += sessions

            case = f"{person.email} on {day}"
            assert len(targets) == edited, (
                f"pages_edited would be {len(targets)}, drew {edited} for {case}"
            )
            assert sessions_total == edits, (
                f"total_edits would be {sessions_total}, drew {edits} for {case}"
            )
            assert len(new_pages) == created, f"pages_created disagrees for {case}"
            checked += 1
            owned.extend(new_pages)

    assert checked > 0, "no author-day was checked"
