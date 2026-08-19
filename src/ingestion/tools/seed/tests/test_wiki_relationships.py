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
    monkeypatch.setattr(wiki, "bulk_insert", capture)
    return captured


@pytest.fixture
def roster() -> list[profiles.Person]:
    return profiles.build_roster("dev@company.nonpresent")


def _column(table: Table, name: str) -> list[Any]:
    cols, rows = table
    index = cols.index(name)
    return [row[index] for row in rows]


def test_each_persons_day_reports_the_pages_the_planner_dated_to_it(
    emitted: Rows, roster: list[profiles.Person]
) -> None:
    pages = wiki._plan_pages(roster, _DAYS)
    assert pages, "the pinned window planned no pages, so this proves nothing"

    wiki.seed_wiki_activity(None, roster, pages, _TENANT, _DAYS)

    planned = Counter((page.author.uuid, page.created_at.date()) for page in pages)
    table = emitted["silver.class_wiki_activity"]
    reported = dict(
        zip(
            zip(_column(table, "author_id"), _column(table, "day"), strict=True),
            _column(table, "pages_created"),
            strict=True,
        )
    )

    for person_day, count in planned.items():
        assert reported.get(person_day) == count, (
            f"{person_day} planned {count} pages and the activity row reports "
            f"{reported.get(person_day)}"
        )
    for person_day, count in reported.items():
        assert count == planned[person_day], (
            f"{person_day} reports {count} pages created and none were planned for it"
        )


def test_every_engagement_row_names_a_planned_page_within_the_window(
    emitted: Rows, roster: list[profiles.Person]
) -> None:
    pages = wiki._plan_pages(roster, _DAYS)
    assert pages, "the pinned window planned no pages, so this proves nothing"

    wiki.seed_wiki_engagement(None, pages, _TENANT, _DAYS)

    window = base.days_window(_DAYS)
    published_on = {page.page_id: page.created_at.date() for page in pages}
    table = emitted["silver.class_wiki_engagement"]
    assert table[1], "the pinned window drew no comments, so this proves nothing"

    for page_id, day in zip(_column(table, "page_id"), _column(table, "day"), strict=True):
        assert page_id in published_on, (
            f"a comment row names page {page_id}, which class_wiki_pages never emitted — "
            f"gold's INNER join drops it"
        )
        assert published_on[page_id] <= day <= window[-1], (
            f"page {page_id} was published on {published_on[page_id]} and drew comments on "
            f"{day}, outside the seeded window ending {window[-1]}"
        )
