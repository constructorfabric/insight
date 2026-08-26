"""
wiki silver-table generator: pages + per-author edits + page comments.

All teams keep documentation, scaled by their profile.
"""

from __future__ import annotations

import datetime as _dt
from collections import Counter
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


def _plan_pages(roster: Sequence[Person], days: int) -> list[_Page]:
    """Every page this run will write, decided before any row is emitted.

    The engagement rows join back onto (tenant_id, source_id, page_id) INNER
    and gold credits the PAGE's author with the comments, so both the comments
    and the activity row's `pages_created` are derived from this list rather
    than drawn independently.
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


def seed_wiki_pages(
    client: clickhouse_connect.driver.client.Client,
    pages: Sequence[_Page],
    tenant_uuid: str,
) -> int:
    truncate(client, "silver", "class_wiki_pages")
    cols = [
        "tenant_id",
        "source_id",
        "unique_key",
        "page_id",
        "space_id",
        "space_name",
        "title",
        "status",
        "author_id",
        "author_email",
        "version_count",
        "created_at",
        "updated_at",
        "source",
        "data_source",
        "collected_at",
        "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    now = anchor_datetime()
    for page in pages:
        rows.append(
            (
                tenant_uuid,
                page.source_id,
                deterministic_uuid("wiki.pages.row", page.page_id),
                page.page_id,
                page.space_id,
                page.space_name,
                page.title,
                "published",
                page.author.uuid,
                page.author.email,
                page.version_count,
                page.created_at,
                page.created_at + _dt.timedelta(minutes=15 * page.version_count),
                SOURCE,
                DATA_SOURCE,
                now,
                version,
            )
        )
    return bulk_insert(client, "silver", "class_wiki_pages", cols, rows)


def seed_wiki_activity(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    pages: Sequence[_Page],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_wiki_activity")
    cols = [
        "tenant_id",
        "source_id",
        "unique_key",
        "author_id",
        "author_email",
        "day",
        "pages_edited",
        "total_edits",
        "pages_created",
        "source",
        "data_source",
        "collected_at",
        "_version",
    ]
    authored = Counter((page.author.uuid, page.created_at.date()) for page in pages)
    rows: list[tuple[object, ...]] = []
    version = 1
    now = anchor_datetime()
    for p in _wiki_authors(roster):
        persona = persona_multiplier(p.uuid)
        team = p.team or ""
        weight = TEAM_PROFILES[team].weights[SOURCE]
        for d in days_window(days):
            rng = seeded_rng(p.uuid, d, "wiki.activity")
            mean = 1.2 * persona * weight * weekday_multiplier(d)
            created = authored[(p.uuid, d)]
            # Publishing a page is itself an edit, so the day's edits can
            # never be fewer than the pages the planner dated to it.
            edits = max(min(poisson(rng, mean), EDITS_CAP), created)
            if edits == 0:
                continue
            edited = max(created, 1, int(edits * rng.uniform(0.4, 0.9)))
            rows.append(
                (
                    tenant_uuid,
                    _source_id(team),
                    deterministic_uuid("wiki.activity.row", p.uuid, d.isoformat()),
                    p.uuid,
                    p.email,
                    d,
                    edited,
                    edits,
                    created,
                    SOURCE,
                    DATA_SOURCE,
                    now,
                    version,
                )
            )
    return bulk_insert(client, "silver", "class_wiki_activity", cols, rows)


def seed_wiki_engagement(
    client: clickhouse_connect.driver.client.Client,
    pages: Sequence[_Page],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_wiki_engagement")
    cols = [
        "tenant_id",
        "source_id",
        "unique_key",
        "page_id",
        "day",
        "total_comments",
        "footer_comments",
        "inline_comments",
        "replies",
        "unique_commenters",
        "source",
        "data_source",
        "collected_at",
        "_version",
    ]
    last_day = days_window(days)[-1]
    rows: list[tuple[object, ...]] = []
    version = 1
    now = anchor_datetime()
    for page in pages:
        published_on = page.created_at.date()
        rng = seeded_rng(page.author.uuid, published_on, f"wiki.comments.{page.page_id}")
        for offset in range(COMMENT_WINDOW_DAYS):
            d = published_on + _dt.timedelta(days=offset)
            if d > last_day:
                break
            total = min(poisson(rng, 0.25 * weekday_multiplier(d)), COMMENTS_CAP)
            if total == 0:
                continue
            footer = rng.randint(0, total)
            rows.append(
                (
                    tenant_uuid,
                    page.source_id,
                    deterministic_uuid("wiki.engagement.row", page.page_id, d.isoformat()),
                    page.page_id,
                    d,
                    total,
                    footer,
                    total - footer,
                    int(total * rng.uniform(0.0, 0.5)),
                    max(1, int(total * rng.uniform(0.5, 1.0))),
                    SOURCE,
                    DATA_SOURCE,
                    now,
                    version,
                )
            )
    return bulk_insert(client, "silver", "class_wiki_engagement", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    pages = _plan_pages(roster, days)
    return {
        "silver.class_wiki_pages": seed_wiki_pages(client, pages, tenant_uuid),
        "silver.class_wiki_activity": seed_wiki_activity(client, roster, pages, tenant_uuid, days),
        "silver.class_wiki_engagement": seed_wiki_engagement(client, pages, tenant_uuid, days),
    }
