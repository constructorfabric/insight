"""
collaboration silver-table generator: meetings + chat + email.

All teams produce some collab activity, scaled by their profile.
"""

from __future__ import annotations

from collections.abc import Sequence
from typing import TYPE_CHECKING

from generators.base import (
    bulk_insert,
    days_window,
    persona_multiplier,
    poisson,
    seeded_rng,
    truncate,
    weekday_multiplier,
)
from profiles import TEAM_PROFILES, Person

if TYPE_CHECKING:
    import clickhouse_connect.driver.client


def _collab_persons(roster: Sequence[Person]) -> list[Person]:
    """Everyone except the CEO contributes collab rows."""
    return [p for p in roster if p.team is not None]


# Gold views filter on `data_source = 'insight_<src>'` (e.g.
# `insight_slack`, `insight_m365`, `insight_zoom`, `insight_gmail`).
# The TEAM_PROFILES keys stay short (slack/m365/...) for readability;
# this lookup adds the prefix at row-emit time.
_DATA_SOURCE_PREFIX = "insight_"


def _meeting_sources(team: str) -> list[tuple[str, float]]:
    """(data_source-as-emitted, weight). Empty if the team has none."""
    out: list[tuple[str, float]] = []
    for src in ("zoom", "m365"):
        w = TEAM_PROFILES[team].weights.get(src, 0)
        if w > 0:
            out.append((_DATA_SOURCE_PREFIX + src, w))
    return out


def _chat_sources(team: str) -> list[tuple[str, float]]:
    out: list[tuple[str, float]] = []
    for src in ("slack", "m365"):
        w = TEAM_PROFILES[team].weights.get(src, 0)
        if w > 0:
            out.append((_DATA_SOURCE_PREFIX + src, w))
    return out


def _email_sources(team: str) -> list[tuple[str, float]]:
    out: list[tuple[str, float]] = []
    for src in ("gmail", "m365"):
        w = TEAM_PROFILES[team].weights.get(src, 0)
        if w > 0:
            out.append((_DATA_SOURCE_PREFIX + src, w))
    return out


def seed_meeting_activity(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_collab_meeting_activity")
    cols = [
        "insight_tenant_id", "email", "person_key", "date", "data_source",
        "meetings_attended", "calls_count", "participants",
        "audio_duration_seconds", "video_duration_seconds",
        "screen_share_duration_seconds", "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in _collab_persons(roster):
        persona = persona_multiplier(p.uuid)
        for source, weight in _meeting_sources(p.team or ""):
            for d in days_window(days):
                rng = seeded_rng(p.uuid, d, f"collab.mtg.{source}")
                mean = 1.6 * persona * weight * weekday_multiplier(d)
                n_meets = min(poisson(rng, mean), 8)
                if n_meets == 0:
                    continue
                # Each meeting 15-60 minutes. Total ≤ 8 hours by construction.
                total_min = min(8 * 60, sum(rng.randint(15, 60) for _ in range(n_meets)))
                audio_s = total_min * 60.0
                video_s = audio_s * 0.7
                share_s = audio_s * 0.2
                rows.append((
                    tenant_uuid, p.email, p.email, d, source,
                    float(n_meets), 0.0,
                    float(n_meets * rng.randint(2, 8)),
                    audio_s, video_s, share_s, version,
                ))
    return bulk_insert(client, "silver", "class_collab_meeting_activity", cols, rows)


def seed_chat_activity(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_collab_chat_activity")
    cols = [
        "insight_tenant_id", "email", "person_key", "date", "data_source",
        "total_chat_messages", "channel_messages_posted_count",
        "channel_posts", "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in _collab_persons(roster):
        persona = persona_multiplier(p.uuid)
        for source, weight in _chat_sources(p.team or ""):
            for d in days_window(days):
                rng = seeded_rng(p.uuid, d, f"collab.chat.{source}")
                mean = 25 * persona * weight * weekday_multiplier(d)
                n_chat = min(poisson(rng, mean), 80)
                if n_chat == 0:
                    continue
                channel = int(n_chat * rng.uniform(0.4, 0.7))
                rows.append((
                    tenant_uuid, p.email, p.email, d, source,
                    float(n_chat), float(channel), float(channel), version,
                ))
    return bulk_insert(client, "silver", "class_collab_chat_activity", cols, rows)


def seed_email_activity(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_collab_email_activity")
    cols = [
        "insight_tenant_id", "email", "person_key", "date", "data_source",
        "sent_count", "received_count", "read_count", "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in _collab_persons(roster):
        persona = persona_multiplier(p.uuid)
        for source, weight in _email_sources(p.team or ""):
            for d in days_window(days):
                rng = seeded_rng(p.uuid, d, f"collab.email.{source}")
                mean_sent = 12 * persona * weight * weekday_multiplier(d)
                sent = min(poisson(rng, mean_sent), 40)
                received = min(poisson(rng, mean_sent * 2.5), 100)
                read = int(received * rng.uniform(0.7, 0.95))
                if sent == 0 and received == 0:
                    continue
                rows.append((
                    tenant_uuid, p.email, p.email, d, source,
                    float(sent), float(received), float(read), version,
                ))
    return bulk_insert(client, "silver", "class_collab_email_activity", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    return {
        "silver.class_collab_meeting_activity":  seed_meeting_activity(client, roster, tenant_uuid, days),
        "silver.class_collab_chat_activity":     seed_chat_activity(client, roster, tenant_uuid, days),
        "silver.class_collab_email_activity":    seed_email_activity(client, roster, tenant_uuid, days),
    }
