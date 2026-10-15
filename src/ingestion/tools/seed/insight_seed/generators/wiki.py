"""Wiki activity, as Outline reports it, into bronze.

Outline delivers documents, their revisions, their comments and the user
directory; `outline__wiki_pages`, `__wiki_activity` and `__wiki_engagement`
derive every wiki measure from those. So this generator emits them and lets
dbt do the counting.

Two consequences of deriving rather than asserting, both deliberate:

* Edits need documents to land on. An author's pages accumulate through the
  window, so the first days would have nothing to edit — hence BACKLOG_PAGES
  per author, created the day BEFORE the window. A wiki older than its
  reporting window is the normal case, and nothing inside the window counts
  them.
* The comment split changed. `outline__wiki_engagement` buckets a comment as
  exactly one of footer / inline / reply, so the three sum to the total; the
  previous generator wrote `footer + inline = total` AND a non-zero `replies`,
  which no set of comments produces. The buckets are now disjoint, so
  class_wiki_engagement's columns moved.
"""

from __future__ import annotations

import datetime as _dt
from collections.abc import Sequence
from dataclasses import dataclass
from typing import TYPE_CHECKING

from ..profiles import TEAM_PROFILES, Person
from .base import (
    anchor_datetime,
    days_window,
    deterministic_uuid,
    persona_multiplier,
    poisson,
    seeded_rng,
    weekday_multiplier,
)
from .insert import bulk_insert, truncate

if TYPE_CHECKING:
    import clickhouse_connect.driver.client


# The TEAM_PROFILES key stays short (outline) for readability; the emitted
# `data_source` carries the `insight_` prefix every connector writes.
SOURCE = "outline"
DATA_SOURCE = "insight_" + SOURCE

PAGES_CAP = 3
EDITS_CAP = 12
COMMENTS_CAP = 6

COMMENT_WINDOW_DAYS = 8

#: Documents an author already had when the window opened. At least EDITS_CAP,
#: so a day's edits always have distinct pages to land on.
BACKLOG_PAGES = EDITS_CAP

#: `outline__wiki_activity` starts a new edit session when two revisions of the
#: same (page, author) are more than 30 minutes apart. Revisions are spaced
#: wider than that so one revision is one session, and the day's first is early
#: enough that EDITS_CAP of them cannot reach midnight.
SESSION_SPACING = _dt.timedelta(minutes=45)
FIRST_REVISION_HOUR = 8

_PAGE_KINDS = ("runbook", "design note", "retro", "onboarding guide", "spec")


@dataclass(frozen=True)
class _Page:
    author: Person
    source_id: str
    page_id: str
    space_id: str
    space_name: str
    title: str
    created_at: _dt.datetime
    version_count: int


def _wiki_authors(roster: Sequence[Person]) -> list[Person]:
    """Everyone on a team with a non-zero outline weight."""
    return [p for p in roster if p.team and TEAM_PROFILES[p.team].weights.get(SOURCE, 0) > 0]


def _source_id(team: str) -> str:
    return deterministic_uuid("wiki.source", team)


def _backlog_pages(roster: Sequence[Person], days: int) -> list[_Page]:
    """The documents each author had before the window opened."""
    opened = days_window(days)[0]
    created_at = _dt.datetime.combine(
        opened - _dt.timedelta(days=1), _dt.time(FIRST_REVISION_HOUR, tzinfo=_dt.UTC)
    )
    return [
        _Page(
            author=person,
            source_id=_source_id(person.team or ""),
            page_id=deterministic_uuid("wiki.page.backlog", person.uuid, str(index)),
            space_id=deterministic_uuid("wiki.space", person.team or ""),
            space_name=f"{person.team} wiki",
            title=f"{person.team} handbook {index + 1}",
            created_at=created_at,
            version_count=1,
        )
        for person in _wiki_authors(roster)
        for index in range(BACKLOG_PAGES)
    ]


def _plan_pages(roster: Sequence[Person], days: int) -> list[_Page]:
    """Every page this run writes, decided before any row is emitted.

    INVARIANT: same rng salt, draw order and caps as before this generator
    wrote bronze — the page set is unchanged.
    """
    pages: list[_Page] = []
    for p in _wiki_authors(roster):
        persona = persona_multiplier(p.uuid)
        team = p.team or ""
        weight = TEAM_PROFILES[team].weights[SOURCE]
        source_id = _source_id(team)
        space_id = deterministic_uuid("wiki.space", team)
        for d in days_window(days):
            rng = seeded_rng(p.uuid, d, "wiki.pages")
            mean = 0.25 * persona * weight * weekday_multiplier(d)
            for i in range(min(poisson(rng, mean), PAGES_CAP)):
                kind = _PAGE_KINDS[rng.randrange(len(_PAGE_KINDS))]
                created_at = _dt.datetime.combine(
                    d,
                    _dt.time(9 + rng.randint(0, 8), rng.randint(0, 59), tzinfo=_dt.UTC),
                )
                pages.append(
                    _Page(
                        author=p,
                        source_id=source_id,
                        page_id=deterministic_uuid("wiki.page", p.uuid, d.isoformat(), str(i)),
                        space_id=space_id,
                        space_name=f"{team} wiki",
                        title=f"{team} {kind} {d.isoformat()}-{i + 1}",
                        created_at=created_at,
                        version_count=rng.randint(1, 12),
                    )
                )
    return pages


def _envelope(source_id: str, tenant_uuid: str) -> dict[str, object]:
    read_at = anchor_datetime()
    return {
        "_airbyte_extracted_at": read_at,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": tenant_uuid,
        "source_id": source_id,
        "data_source": DATA_SOURCE,
        "collected_at": read_at.isoformat(),
    }


def _positioned(cols: list[str], fields: dict[str, object]) -> tuple[object, ...]:
    return tuple(fields.get(col) for col in cols)


AIRBYTE_COLS = [
    "_airbyte_raw_id",
    "_airbyte_extracted_at",
    "_airbyte_meta",
    "_airbyte_generation_id",
]
IDENTITY_COLS = ["unique_key", "tenant_id", "source_id", "data_source", "collected_at"]


def seed_wiki_users(
    client: clickhouse_connect.driver.client.Client, roster: Sequence[Person], tenant_uuid: str
) -> int:
    """The directory the staging models resolve an author id to an email through."""
    truncate(client, "bronze_outline", "wiki_users")
    cols = [
        *AIRBYTE_COLS,
        *IDENTITY_COLS,
        "user_id",
        "name",
        "email",
        "role",
        "is_suspended",
        "last_active_at",
        "created_at",
    ]
    rows = []
    for person in _wiki_authors(roster):
        source_id = _source_id(person.team or "")
        rows.append(
            _positioned(
                cols,
                {
                    **_envelope(source_id, tenant_uuid),
                    "_airbyte_raw_id": deterministic_uuid("wiki.user.raw", person.uuid),
                    "unique_key": f"{tenant_uuid}:{source_id}:{person.uuid}",
                    "user_id": person.uuid,
                    "name": person.display_name,
                    "email": person.email,
                    "role": "member",
                    "is_suspended": 0,
                    "last_active_at": anchor_datetime().isoformat(),
                    "created_at": anchor_datetime().isoformat(),
                },
            )
        )
    return bulk_insert(client, "bronze_outline", "wiki_users", cols, rows)


def seed_wiki_spaces(
    client: clickhouse_connect.driver.client.Client, pages: Sequence[_Page], tenant_uuid: str
) -> int:
    truncate(client, "bronze_outline", "wiki_spaces")
    cols = [
        *AIRBYTE_COLS,
        *IDENTITY_COLS,
        "space_id",
        "name",
        "description",
        "space_type",
        "status",
        "url",
        "created_at",
    ]
    seen: dict[tuple[str, str], _Page] = {}
    for page in pages:
        seen.setdefault((page.source_id, page.space_id), page)
    rows = [
        _positioned(
            cols,
            {
                **_envelope(source_id, tenant_uuid),
                "_airbyte_raw_id": deterministic_uuid("wiki.space.raw", space_id),
                "unique_key": f"{tenant_uuid}:{source_id}:{space_id}",
                "space_id": space_id,
                "name": page.space_name,
                "description": f"Synthetic space {page.space_name}",
                "space_type": "collection",
                "status": "active",
                "url": f"/collection/{space_id}",
                "created_at": anchor_datetime().isoformat(),
            },
        )
        for (source_id, space_id), page in seen.items()
    ]
    return bulk_insert(client, "bronze_outline", "wiki_spaces", cols, rows)


def seed_wiki_pages(
    client: clickhouse_connect.driver.client.Client, pages: Sequence[_Page], tenant_uuid: str
) -> int:
    truncate(client, "bronze_outline", "wiki_pages")
    cols = [
        *AIRBYTE_COLS,
        *IDENTITY_COLS,
        "page_id",
        "space_id",
        "title",
        "status",
        "author_id",
        "author_email",
        "last_editor_id",
        "last_editor_email",
        "created_at",
        "updated_at",
        "version_number",
        "parent_page_id",
    ]
    rows = [
        _positioned(
            cols,
            {
                **_envelope(page.source_id, tenant_uuid),
                "_airbyte_raw_id": deterministic_uuid("wiki.page.raw", page.page_id),
                "unique_key": f"{tenant_uuid}:{page.source_id}:{page.page_id}",
                "page_id": page.page_id,
                "space_id": page.space_id,
                "title": page.title,
                "status": "published",
                "author_id": page.author.uuid,
                # Outline's documents endpoint omits the email on self-hosted
                # instances; the staging model resolves it through wiki_users.
                "author_email": "",
                "last_editor_id": page.author.uuid,
                "last_editor_email": "",
                "created_at": page.created_at.isoformat(),
                "updated_at": (
                    page.created_at + _dt.timedelta(minutes=15 * page.version_count)
                ).isoformat(),
                "version_number": page.version_count,
                "parent_page_id": "",
            },
        )
        for page in pages
    ]
    return bulk_insert(client, "bronze_outline", "wiki_pages", cols, rows)


def _revision_times(day: _dt.date, sessions: int) -> list[_dt.datetime]:
    """One timestamp per edit session, spaced past the session-collapse gap."""
    start = _dt.datetime.combine(day, _dt.time(FIRST_REVISION_HOUR, tzinfo=_dt.UTC))
    return [start + SESSION_SPACING * index for index in range(sessions)]


def seed_wiki_page_versions(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    backlog: Sequence[_Page],
    planned: Sequence[_Page],
    tenant_uuid: str,
    days: int,
) -> int:
    """The revisions pages_created, total_edits and pages_edited are counted from."""
    truncate(client, "bronze_outline", "wiki_page_versions")
    cols = [
        *AIRBYTE_COLS,
        *IDENTITY_COLS,
        "page_id",
        "revision_id",
        "author_id",
        "author_email",
        "created_at",
        "message",
    ]

    by_author_day: dict[tuple[str, _dt.date], list[_Page]] = {}
    for page in planned:
        by_author_day.setdefault((page.author.uuid, page.created_at.date()), []).append(page)
    backlog_by_author: dict[str, list[_Page]] = {}
    for page in backlog:
        backlog_by_author.setdefault(page.author.uuid, []).append(page)

    rows: list[tuple[object, ...]] = []

    def emit(page: _Page, author: Person, at: _dt.datetime, index: int) -> None:
        revision = deterministic_uuid("wiki.revision", page.page_id, at.isoformat(), str(index))
        rows.append(
            _positioned(
                cols,
                {
                    **_envelope(page.source_id, tenant_uuid),
                    "_airbyte_raw_id": deterministic_uuid("wiki.revision.raw", revision),
                    "unique_key": f"{tenant_uuid}:{page.source_id}:{revision}",
                    "page_id": page.page_id,
                    "revision_id": revision,
                    "author_id": author.uuid,
                    # revisions.list embeds the author without an email; the
                    # staging model resolves it through wiki_users.
                    "author_email": "",
                    "created_at": at.isoformat(),
                    "message": "",
                },
            )
        )

    # The backlog's own first revision, before the window: it is what makes an
    # in-window edit of a backlog page an edit rather than a creation.
    for page in backlog:
        emit(page, page.author, page.created_at, 0)

    for person in _wiki_authors(roster):
        persona = persona_multiplier(person.uuid)
        weight = TEAM_PROFILES[person.team or ""].weights[SOURCE]
        owned = sorted(backlog_by_author.get(person.uuid, []), key=lambda page: page.page_id)
        for day_index, day in enumerate(days_window(days)):
            rng = seeded_rng(person.uuid, day, "wiki.activity")
            mean = 1.2 * persona * weight * weekday_multiplier(day)
            new_pages = by_author_day.get((person.uuid, day), [])
            created = len(new_pages)
            # Publishing a page is itself an edit, so the day's edits can
            # never be fewer than the pages the planner dated to it.
            edits = max(min(poisson(rng, mean), EDITS_CAP), created)
            if edits == 0:
                owned.extend(new_pages)
                continue
            edited = max(created, 1, int(edits * rng.uniform(0.4, 0.9)))

            # The day's new pages first — their first revision is what makes
            # them count as created — then older ones, rotated so a run does
            # not edit the same handful every day.
            older = [page for page in owned if page.created_at.date() < day]
            offset = day_index % max(len(older), 1)
            rotated = older[offset:] + older[:offset]
            targets = [*new_pages, *rotated][:edited]

            for position, page in enumerate(targets):
                sessions = 1 + (edits - edited + len(targets) - 1 - position) // len(targets)
                for index, at in enumerate(_revision_times(day, sessions)):
                    emit(page, person, at, index)

            owned.extend(new_pages)

    return bulk_insert(client, "bronze_outline", "wiki_page_versions", cols, rows)


def seed_wiki_comments(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    pages: Sequence[_Page],
    tenant_uuid: str,
    days: int,
) -> int:
    """The comments the engagement counts are grouped from.

    A comment is exactly one of footer / inline / reply, because that is how
    `outline__wiki_engagement` buckets it — see the module docstring.
    """
    truncate(client, "bronze_outline", "wiki_comments")
    cols = [
        *AIRBYTE_COLS,
        *IDENTITY_COLS,
        "comment_id",
        "page_id",
        "parent_comment_id",
        "author_id",
        "author_email",
        "created_at",
        "updated_at",
        "resolution_status",
        "anchor_text",
    ]
    commenters = _wiki_authors(roster)
    last_day = days_window(days)[-1]
    rows: list[tuple[object, ...]] = []
    for page in pages:
        published_on = page.created_at.date()
        rng = seeded_rng(page.author.uuid, published_on, f"wiki.comments.{page.page_id}")
        for offset in range(COMMENT_WINDOW_DAYS):
            day = published_on + _dt.timedelta(days=offset)
            if day > last_day:
                break
            total = min(poisson(rng, 0.25 * weekday_multiplier(day)), COMMENTS_CAP)
            if total == 0:
                continue
            replies = int(total * rng.uniform(0.0, 0.5))
            footer = rng.randint(0, total - replies)
            voices = max(1, int(total * rng.uniform(0.5, 1.0)))
            at = _dt.datetime.combine(day, _dt.time(11, tzinfo=_dt.UTC))
            for index in range(total):
                comment_id = deterministic_uuid(
                    "wiki.comment", page.page_id, day.isoformat(), str(index)
                )
                is_reply = index < replies
                is_inline = not is_reply and index >= replies + footer
                # Round-robin over `voices` of them, so uniqExact(author_id) is that many.
                author = commenters[index % voices]
                rows.append(
                    _positioned(
                        cols,
                        {
                            **_envelope(page.source_id, tenant_uuid),
                            "_airbyte_raw_id": deterministic_uuid("wiki.comment.raw", comment_id),
                            "unique_key": f"{tenant_uuid}:{page.source_id}:{comment_id}",
                            "comment_id": comment_id,
                            "page_id": page.page_id,
                            # The first comment that is NOT a reply. Pointing
                            # every reply at index 0 made the first one its own
                            # parent, since the replies lead the day's run.
                            "parent_comment_id": (
                                deterministic_uuid(
                                    "wiki.comment", page.page_id, day.isoformat(), str(replies)
                                )
                                if is_reply
                                else ""
                            ),
                            "author_id": author.uuid,
                            "author_email": "",
                            "created_at": at.isoformat(),
                            "updated_at": at.isoformat(),
                            "resolution_status": "open",
                            "anchor_text": "the paragraph above" if is_inline else "",
                        },
                    )
                )
    return bulk_insert(client, "bronze_outline", "wiki_comments", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    # Staging is a view here, but silver carries a _version watermark: rows a
    # previous run left at another anchor would never be revisited.
    truncate(client, "silver", "class_wiki_pages")
    truncate(client, "silver", "class_wiki_activity")
    truncate(client, "silver", "class_wiki_engagement")

    planned = _plan_pages(roster, days)
    backlog = _backlog_pages(roster, days)
    pages = [*backlog, *planned]
    return {
        "bronze_outline.wiki_users": seed_wiki_users(client, roster, tenant_uuid),
        "bronze_outline.wiki_spaces": seed_wiki_spaces(client, pages, tenant_uuid),
        "bronze_outline.wiki_pages": seed_wiki_pages(client, pages, tenant_uuid),
        "bronze_outline.wiki_page_versions": seed_wiki_page_versions(
            client, roster, backlog, planned, tenant_uuid, days
        ),
        "bronze_outline.wiki_comments": seed_wiki_comments(
            client, roster, planned, tenant_uuid, days
        ),
    }
