"""The two joins the wiki generator's plan-first design exists to hold.

`_plan_pages` decides every page before a row is written precisely because two
downstream relations are derived from that list rather than drawn beside it:
the activity row's `pages_created`, and the engagement rows, which gold joins
back onto `(tenant_id, source_id, page_id)` INNER and credits to the PAGE's
author. A count drawn independently would either double-count a person's day or
strand comments on a page that was never emitted, and gold would answer with a
number nobody can trace to a page.

Run against the installed package (see the README's develop section):

    uv run --extra dev pytest tests
"""

from __future__ import annotations

import datetime as _dt
from collections import Counter
from typing import Any

import pytest

from insight_seed import profiles
from insight_seed.generators import base, wiki

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

    monkeypatch.setattr(wiki, "truncate", lambda client, schema, table: None)

    monkeypatch.setattr(wiki, "prune_to_bronze", lambda client, snapshot, tenant: None)
    monkeypatch.setattr(wiki, "bulk_insert", capture)
    return captured


@pytest.fixture
def roster() -> list[profiles.Person]:
    return profiles.build_roster("dev@company.nonpresent")


def _column(table: Table, name: str) -> list[Any]:
    cols, rows = table
    index = cols.index(name)
    return [row[index] for row in rows]


def test_each_persons_day_creates_the_pages_the_planner_dated_to_it(
    emitted: Rows, roster: list[profiles.Person]
) -> None:
    """`pages_created` is countIf(revision_ordinal = 1), so a planned page must
    get its FIRST revision on the day the planner dated it — and no other
    page's first revision may land on that day."""
    planned = wiki._plan_pages(roster, _DAYS)
    backlog = wiki._backlog_pages(roster, _DAYS)
    assert planned, "the pinned window planned no pages, so this proves nothing"

    wiki.seed_wiki_page_versions(None, roster, backlog, planned, _TENANT, _DAYS)

    table = emitted["bronze_outline.wiki_page_versions"]
    first_revision: dict[str, tuple[_dt.date, str]] = {}
    for page_id, author_id, created_at in zip(
        _column(table, "page_id"),
        _column(table, "author_id"),
        _column(table, "created_at"),
        strict=True,
    ):
        at = _dt.datetime.fromisoformat(created_at)
        if page_id not in first_revision or at.date() < first_revision[page_id][0]:
            first_revision[page_id] = (at.date(), author_id)

    created = Counter((author_id, day) for day, author_id in first_revision.values())
    expected = Counter((page.author.uuid, page.created_at.date()) for page in planned)
    expected.update(Counter((page.author.uuid, page.created_at.date()) for page in backlog))

    assert created == expected, (
        "the revisions that would count as page creations do not match the pages "
        "the planner dated to each person-day"
    )


def test_every_comment_names_a_planned_page_within_the_window(
    emitted: Rows, roster: list[profiles.Person]
) -> None:
    pages = wiki._plan_pages(roster, _DAYS)
    assert pages, "the pinned window planned no pages, so this proves nothing"

    wiki.seed_wiki_comments(None, roster, pages, _TENANT, _DAYS)

    window = base.days_window(_DAYS)
    published_on = {page.page_id: page.created_at.date() for page in pages}
    table = emitted["bronze_outline.wiki_comments"]
    assert table[1], "the pinned window drew no comments, so this proves nothing"

    for page_id, created_at in zip(
        _column(table, "page_id"), _column(table, "created_at"), strict=True
    ):
        day = _dt.datetime.fromisoformat(created_at).date()
        assert page_id in published_on, (
            f"a comment names page {page_id}, which wiki_pages never emitted — "
            f"gold's INNER join drops it"
        )
        assert published_on[page_id] <= day <= window[-1], (
            f"page {page_id} was published on {published_on[page_id]} and drew comments on "
            f"{day}, outside the seeded window ending {window[-1]}"
        )
