"""AI tool usage, as each vendor reports it, into bronze.

Four connectors carry what the two class relations union:

* Cursor and Claude Team report developer usage — Cursor as one daily row per
  active user, Claude Team as a per-email code-metrics row.
* ChatGPT Team and Claude Enterprise report assistant usage, the first as a
  seat plus a daily chat-activity row, the second as one row per user per day.

Two measures cannot survive the move, because the connector decides them:

* Cursor reports one row per user per day and the staging model stamps
  `session_count` as 1. The drawn session count still shapes the day's
  requests, accepts and lines — it just is not reported as a session count.
* The assistant surface counters the generator invented (voice, canvas and
  friends) are not in either vendor's stream, so they are no longer stated.
"""

from __future__ import annotations

import datetime as _dt
from collections.abc import Sequence
from typing import TYPE_CHECKING, NamedTuple

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


_DEV_TOOLS = (
    ("cursor", "cursor"),
    ("claude_code", "claude_team"),
)
_ASSISTANT_TOOLS = (
    ("chatgpt", "chat", "chatgpt"),
    ("claude", "chat", "claude_team"),
)

CURSOR_SOURCE_ID = deterministic_uuid("ai.source", "cursor")
CLAUDE_TEAM_SOURCE_ID = deterministic_uuid("ai.source", "claude_team")
CLAUDE_ENTERPRISE_SOURCE_ID = deterministic_uuid("ai.source", "claude_enterprise")
CHATGPT_SOURCE_ID = deterministic_uuid("ai.source", "chatgpt_team")

AIRBYTE_COLS = [
    "_airbyte_raw_id",
    "_airbyte_extracted_at",
    "_airbyte_meta",
    "_airbyte_generation_id",
]


class DevDay(NamedTuple):
    sessions: int
    offered: int
    accepted: int
    lines_added: int
    lines_removed: int
    cost: float


class AssistantDay(NamedTuple):
    sessions: int
    messages: int
    conversations: int


def _measured(roster: Sequence[Person]) -> list[Person]:
    return [p for p in roster if p.team]


def _dev_day(person: Person, day: _dt.date, tool: str, weight: float) -> DevDay:
    """INVARIANT: same salt and draw order as before this generator wrote
    bronze — `poisson` and the uniforms consume rng state in sequence."""
    rng = seeded_rng(person.uuid, day, f"ai.dev.{tool}")
    base = 6 * persona_multiplier(person.uuid) * weight * weekday_multiplier(day)
    sessions = min(poisson(rng, base), 30)
    if sessions == 0:
        return DevDay(0, 0, 0, 0, 0, 0.0)
    offered = sessions * rng.randint(3, 8)
    accepted = int(offered * rng.uniform(0.4, 0.85))
    lines_added = min(int(accepted * rng.randint(3, 18)), 400)
    lines_removed = int(lines_added * rng.uniform(0.2, 0.8))
    return DevDay(
        sessions, offered, accepted, lines_added, lines_removed, sessions * rng.uniform(2.0, 12.0)
    )


def _assistant_day(person: Person, day: _dt.date, tool: str, weight: float) -> AssistantDay:
    rng = seeded_rng(person.uuid, day, f"ai.assistant.{tool}")
    base = 3 * persona_multiplier(person.uuid) * weight * weekday_multiplier(day)
    sessions = min(poisson(rng, base), 20)
    if sessions == 0:
        return AssistantDay(0, 0, 0)
    messages = sessions * rng.randint(4, 14)
    return AssistantDay(sessions, messages, max(1, int(sessions * rng.uniform(0.6, 1.0))))


def _weight(person: Person, key: str) -> float:
    return TEAM_PROFILES[person.team or ""].weights.get(key, 0)


def _positioned(cols: list[str], fields: dict[str, object]) -> tuple[object, ...]:
    return tuple(fields.get(col) for col in cols)


def _envelope(source_id: str, tenant_uuid: str) -> dict[str, object]:
    read_at = anchor_datetime()
    return {
        "_airbyte_extracted_at": read_at,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": tenant_uuid,
        "source_id": source_id,
        "collected_at": read_at.isoformat(),
    }


def seed_cursor_daily_usage(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "bronze_cursor", "cursor_daily_usage")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "userId",
        "email",
        "day",
        "date",
        "isActive",
        "chatRequests",
        "composerRequests",
        "agentRequests",
        "totalTabsShown",
        "totalTabsAccepted",
        "totalAccepts",
        "totalRejects",
        "totalLinesAdded",
        "totalLinesDeleted",
        "acceptedLinesAdded",
        "acceptedLinesDeleted",
        "mostUsedModel",
    ]
    rows: list[tuple[object, ...]] = []
    for person in _measured(roster):
        weight = _weight(person, "cursor")
        if weight <= 0:
            continue
        for day in days_window(days):
            drawn = _dev_day(person, day, "cursor", weight)
            if drawn.sessions == 0:
                continue
            key = f"{tenant_uuid}:{CURSOR_SOURCE_ID}:{person.uuid}:{day.isoformat()}"
            rows.append(
                _positioned(
                    cols,
                    {
                        **_envelope(CURSOR_SOURCE_ID, tenant_uuid),
                        "_airbyte_raw_id": deterministic_uuid("ai.cursor.raw", key),
                        "unique_key": key,
                        "userId": person.uuid,
                        "email": person.email,
                        "day": day.isoformat(),
                        # Cursor stamps the day as epoch milliseconds.
                        "date": int(
                            _dt.datetime.combine(day, _dt.time(), tzinfo=_dt.UTC).timestamp() * 1000
                        ),
                        "isActive": True,
                        "chatRequests": drawn.sessions,
                        "composerRequests": drawn.sessions,
                        "agentRequests": 0,
                        "totalTabsShown": drawn.offered,
                        "totalTabsAccepted": drawn.accepted,
                        "totalAccepts": drawn.accepted,
                        "totalRejects": drawn.offered - drawn.accepted,
                        "totalLinesAdded": drawn.lines_added,
                        "totalLinesDeleted": drawn.lines_removed,
                        "acceptedLinesAdded": drawn.lines_added,
                        "acceptedLinesDeleted": drawn.lines_removed,
                        "mostUsedModel": "claude-sonnet-5-5",
                    },
                )
            )
    return bulk_insert(client, "bronze_cursor", "cursor_daily_usage", cols, rows)


def seed_claude_team_code_metrics(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    """Claude Code's per-email metrics, plus the org roll-up the model joins."""
    truncate(client, "bronze_claude_team", "claude_team_code_metrics")
    truncate(client, "bronze_claude_team", "claude_team_code_metrics_org")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "data_source",
        "metric_date",
        "email",
        "api_key_name",
        "status",
        "total_sessions",
        "total_lines_accepted",
        "avg_lines_accepted_per_day",
        "total_cost",
        "avg_cost_per_day",
        "last_active",
        "prs_with_cc",
        "total_prs",
    ]
    org_cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "data_source",
        "metric_date",
        "total_users",
    ]
    rows: list[tuple[object, ...]] = []
    org: dict[_dt.date, int] = {}
    for person in _measured(roster):
        weight = _weight(person, "claude_team")
        if weight <= 0:
            continue
        for day in days_window(days):
            drawn = _dev_day(person, day, "claude_code", weight)
            if drawn.sessions == 0:
                continue
            org[day] = org.get(day, 0) + 1
            key = f"{tenant_uuid}:{CLAUDE_TEAM_SOURCE_ID}:{person.email}:{day.isoformat()}"
            rows.append(
                _positioned(
                    cols,
                    {
                        **_envelope(CLAUDE_TEAM_SOURCE_ID, tenant_uuid),
                        "_airbyte_raw_id": deterministic_uuid("ai.ccode.raw", key),
                        "unique_key": key,
                        "data_source": "insight_claude_team",
                        "metric_date": day.isoformat(),
                        "email": person.email,
                        "api_key_name": "",
                        "status": "active",
                        "total_sessions": drawn.sessions,
                        "total_lines_accepted": drawn.lines_added,
                        "avg_lines_accepted_per_day": drawn.lines_added,
                        # The stream carries money as text.
                        "total_cost": f"{drawn.cost:.2f}",
                        "avg_cost_per_day": f"{drawn.cost:.2f}",
                        "last_active": day.isoformat(),
                        "prs_with_cc": 0,
                        "total_prs": 0,
                    },
                )
            )
    org_rows = [
        _positioned(
            org_cols,
            {
                **_envelope(CLAUDE_TEAM_SOURCE_ID, tenant_uuid),
                "_airbyte_raw_id": deterministic_uuid("ai.ccode.org.raw", day.isoformat()),
                "unique_key": f"{tenant_uuid}:{CLAUDE_TEAM_SOURCE_ID}:org:{day.isoformat()}",
                "data_source": "insight_claude_team",
                "metric_date": day.isoformat(),
                "total_users": users,
            },
        )
        for day, users in sorted(org.items())
    ]
    return {
        "bronze_claude_team.claude_team_code_metrics": bulk_insert(
            client, "bronze_claude_team", "claude_team_code_metrics", cols, rows
        ),
        "bronze_claude_team.claude_team_code_metrics_org": bulk_insert(
            client, "bronze_claude_team", "claude_team_code_metrics_org", org_cols, org_rows
        ),
    }


def seed_chatgpt_team(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    """A seat per person, and a chat-activity row per day they used it."""
    truncate(client, "bronze_chatgpt_team", "chatgpt_team_seats")
    truncate(client, "bronze_chatgpt_team", "chatgpt_team_chat_activity")
    seat_cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "data_source",
        "user_id",
        "email",
        "name",
        "role",
        "seat_type",
        "added_at",
    ]
    activity_cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "data_source",
        "date",
        "email",
        "name",
        "seat_type",
        "messages",
        "gpt_messages",
        "tool_messages",
        "project_messages",
        "credits_used",
    ]
    seats: list[tuple[object, ...]] = []
    activity: list[tuple[object, ...]] = []
    for person in _measured(roster):
        weight = _weight(person, "chatgpt")
        if weight <= 0:
            continue
        seats.append(
            _positioned(
                seat_cols,
                {
                    **_envelope(CHATGPT_SOURCE_ID, tenant_uuid),
                    "_airbyte_raw_id": deterministic_uuid("ai.chatgpt.seat.raw", person.uuid),
                    "unique_key": f"{tenant_uuid}:{CHATGPT_SOURCE_ID}:{person.uuid}",
                    "data_source": "insight_chatgpt_team",
                    "user_id": person.uuid,
                    "email": person.email,
                    "name": person.display_name,
                    "role": "member",
                    "seat_type": "standard",
                    "added_at": anchor_datetime().isoformat(),
                },
            )
        )
        for day in days_window(days):
            drawn = _assistant_day(person, day, "chatgpt", weight)
            if drawn.sessions == 0:
                continue
            key = f"{tenant_uuid}:{CHATGPT_SOURCE_ID}:{person.email}:{day.isoformat()}"
            activity.append(
                _positioned(
                    activity_cols,
                    {
                        **_envelope(CHATGPT_SOURCE_ID, tenant_uuid),
                        "_airbyte_raw_id": deterministic_uuid("ai.chatgpt.raw", key),
                        "unique_key": key,
                        "data_source": "insight_chatgpt_team",
                        "date": day.isoformat(),
                        "email": person.email,
                        "name": person.display_name,
                        "seat_type": "standard",
                        "messages": drawn.messages,
                        "gpt_messages": drawn.messages,
                        "tool_messages": 0,
                        "project_messages": 0,
                        "credits_used": drawn.sessions,
                    },
                )
            )
    return {
        "bronze_chatgpt_team.chatgpt_team_seats": bulk_insert(
            client, "bronze_chatgpt_team", "chatgpt_team_seats", seat_cols, seats
        ),
        "bronze_chatgpt_team.chatgpt_team_chat_activity": bulk_insert(
            client, "bronze_chatgpt_team", "chatgpt_team_chat_activity", activity_cols, activity
        ),
    }


def seed_claude_enterprise_users(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    """Claude's own per-user daily counters, the `claude` assistant arm."""
    truncate(client, "bronze_claude_enterprise", "claude_enterprise_users")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "data_source",
        "date",
        "user_id",
        "user_email",
        "chat_conversation_count",
        "chat_message_count",
        "chat_thinking_message_count",
    ]
    rows: list[tuple[object, ...]] = []
    for person in _measured(roster):
        weight = _weight(person, "claude_team")
        if weight <= 0:
            continue
        for day in days_window(days):
            drawn = _assistant_day(person, day, "claude", weight)
            if drawn.sessions == 0:
                continue
            key = f"{tenant_uuid}:{CLAUDE_ENTERPRISE_SOURCE_ID}:{person.email}:{day.isoformat()}"
            rows.append(
                _positioned(
                    cols,
                    {
                        **_envelope(CLAUDE_ENTERPRISE_SOURCE_ID, tenant_uuid),
                        "_airbyte_raw_id": deterministic_uuid("ai.claude.raw", key),
                        "unique_key": key,
                        "data_source": "insight_claude_enterprise",
                        "date": day.isoformat(),
                        "user_id": person.uuid,
                        "user_email": person.email,
                        "chat_conversation_count": drawn.conversations,
                        "chat_message_count": drawn.messages,
                        "chat_thinking_message_count": drawn.messages // 3,
                    },
                )
            )
    return bulk_insert(client, "bronze_claude_enterprise", "claude_enterprise_users", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    truncate(client, "staging", "cursor__ai_dev_usage")
    truncate(client, "staging", "claude_team__ai_dev_usage")
    truncate(client, "staging", "chatgpt_team__ai_assistant_usage")
    truncate(client, "staging", "claude_enterprise__ai_assistant_usage")
    truncate(client, "silver", "class_ai_dev_usage")
    truncate(client, "silver", "class_ai_assistant_usage")

    return {
        "bronze_cursor.cursor_daily_usage": seed_cursor_daily_usage(
            client, roster, tenant_uuid, days
        ),
        **seed_claude_team_code_metrics(client, roster, tenant_uuid, days),
        **seed_chatgpt_team(client, roster, tenant_uuid, days),
        "bronze_claude_enterprise.claude_enterprise_users": seed_claude_enterprise_users(
            client, roster, tenant_uuid, days
        ),
    }
