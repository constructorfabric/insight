"""Collaboration activity, as its vendors report it, into bronze.

Three connectors carry it and the class relations union them:

* Microsoft 365 reports one usage row per licensed user per day. `teams_activity`
  carries both the chat counts and the meeting counts, so one bronze row feeds
  `m365__collab_chat_activity` and `m365__collab_meeting_activity` alike;
  `email_activity` feeds the third.
* Slack reports the same shape on `users_details`.
* Zoom reports meetings and their participants, and the staging model derives
  attendance and presence from the join/leave spans.

Two things moved, both because a connector decides them and the generator no
longer can:

* `participants` is a Zoom column. Microsoft's report does not carry it, so an
  m365 meeting row no longer states one.
* Email from Google is gone. There is no Gmail connector in the tree, so a
  gmail-sourced row is a row no install could ever hold. Every team keeps its
  m365 email activity; `gmail` survives in TEAM_PROFILES for whenever that
  connector lands.
"""

from __future__ import annotations

import datetime as _dt
from collections.abc import Sequence
from typing import TYPE_CHECKING, NamedTuple

from ..profiles import TEAM_PROFILES, Person
from .base import (
    anchor_datetime,
    days_window,
    deterministic_int,
    deterministic_uuid,
    persona_multiplier,
    poisson,
    seeded_rng,
    weekday_multiplier,
)
from .insert import bulk_insert, truncate
from .snapshots import prune_to_bronze

if TYPE_CHECKING:
    import clickhouse_connect.driver.client


_DATA_SOURCE_PREFIX = "insight_"

M365_SOURCE_ID = deterministic_uuid("collab.source", "m365")
SLACK_SOURCE_ID = deterministic_uuid("collab.source", "slack")
ZOOM_SOURCE_ID = deterministic_uuid("collab.source", "zoom")

AIRBYTE_COLS = [
    "_airbyte_raw_id",
    "_airbyte_extracted_at",
    "_airbyte_meta",
    "_airbyte_generation_id",
]


def _collab_persons(roster: Sequence[Person]) -> list[Person]:
    """Everyone except the CEO contributes collab rows."""
    return [p for p in roster if p.team is not None]


def _weight(team: str, source: str) -> float:
    return TEAM_PROFILES[team].weights.get(source, 0)


def _positioned(cols: list[str], fields: dict[str, object]) -> tuple[object, ...]:
    return tuple(fields.get(col) for col in cols)


def _envelope(source_id: str, tenant_uuid: str) -> dict[str, object]:
    return {
        "_airbyte_extracted_at": anchor_datetime(),
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": tenant_uuid,
        "source_id": source_id,
    }


class Meetings(NamedTuple):
    count: int
    participants: int
    audio_seconds: float


class Chat(NamedTuple):
    total: int
    channel: int


class Email(NamedTuple):
    sent: int
    received: int
    read: int


def _meetings(person: Person, day: _dt.date, source: str) -> Meetings:
    """INVARIANT: same salt, draws and caps as before this generator wrote
    bronze — a day's meetings did not move."""
    weight = _weight(person.team or "", source)
    if weight <= 0:
        return Meetings(0, 0, 0.0)
    rng = seeded_rng(person.uuid, day, f"collab.mtg.{_DATA_SOURCE_PREFIX}{source}")
    mean = 1.6 * persona_multiplier(person.uuid) * weight * weekday_multiplier(day)
    count = min(poisson(rng, mean), 8)
    if count == 0:
        return Meetings(0, 0, 0.0)
    # Each meeting 15-60 minutes. Total <= 8 hours by construction.
    minutes = min(8 * 60, sum(rng.randint(15, 60) for _ in range(count)))
    return Meetings(count, count * rng.randint(2, 8), minutes * 60.0)


def _chat(person: Person, day: _dt.date, source: str) -> Chat:
    weight = _weight(person.team or "", source)
    if weight <= 0:
        return Chat(0, 0)
    rng = seeded_rng(person.uuid, day, f"collab.chat.{_DATA_SOURCE_PREFIX}{source}")
    mean = 25 * persona_multiplier(person.uuid) * weight * weekday_multiplier(day)
    total = min(poisson(rng, mean), 80)
    if total == 0:
        return Chat(0, 0)
    return Chat(total, int(total * rng.uniform(0.4, 0.7)))


def _email(person: Person, day: _dt.date, source: str) -> Email:
    weight = _weight(person.team or "", source)
    if weight <= 0:
        return Email(0, 0, 0)
    rng = seeded_rng(person.uuid, day, f"collab.email.{_DATA_SOURCE_PREFIX}{source}")
    mean_sent = 12 * persona_multiplier(person.uuid) * weight * weekday_multiplier(day)
    sent = min(poisson(rng, mean_sent), 40)
    received = min(poisson(rng, mean_sent * 2.5), 100)
    return Email(sent, received, int(received * rng.uniform(0.7, 0.95)))


def seed_m365_teams_activity(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    """One usage row per licensed user per day, carrying chat AND meetings.

    Microsoft reports both on the same row, which is why a day with only one
    kind of activity still reports the other as zero.
    """
    truncate(client, "bronze_m365", "teams_activity")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "reportRefreshDate",
        "reportPeriod",
        "userPrincipalName",
        "userId",
        "isLicensed",
        "isDeleted",
        "lastActivityDate",
        "assignedProducts",
        "privateChatMessageCount",
        "teamChatMessageCount",
        "postMessages",
        "replyMessages",
        "urgentMessages",
        "callCount",
        "meetingCount",
        "meetingsAttendedCount",
        "meetingsOrganizedCount",
        "audioDuration",
        "videoDuration",
        "screenShareDuration",
    ]
    rows: list[tuple[object, ...]] = []
    for person in _collab_persons(roster):
        for day in days_window(days):
            chat = _chat(person, day, "m365")
            meetings = _meetings(person, day, "m365")
            if chat.total == 0 and meetings.count == 0:
                continue
            key = f"{tenant_uuid}:{M365_SOURCE_ID}:{person.email}:{day.isoformat()}"
            rows.append(
                _positioned(
                    cols,
                    {
                        **_envelope(M365_SOURCE_ID, tenant_uuid),
                        "_airbyte_raw_id": deterministic_uuid("collab.m365.teams.raw", key),
                        "unique_key": key,
                        "reportRefreshDate": day.isoformat(),
                        "reportPeriod": "1",
                        "userPrincipalName": person.email,
                        "userId": person.uuid,
                        "isLicensed": True,
                        "isDeleted": False,
                        # SAFETY: m365_reported_days drops a whole day when any
                        # row shows activity after its own lastActivityDate —
                        # Graph's signature for a report it never computed.
                        "lastActivityDate": day.isoformat(),
                        "assignedProducts": '["TEAMS"]',
                        "privateChatMessageCount": chat.total - chat.channel,
                        "teamChatMessageCount": chat.channel,
                        "postMessages": chat.channel,
                        "replyMessages": 0,
                        "urgentMessages": 0,
                        "callCount": 0,
                        "meetingCount": meetings.count,
                        "meetingsAttendedCount": meetings.count,
                        "meetingsOrganizedCount": 0,
                        "audioDuration": f"PT{int(meetings.audio_seconds)}S",
                        "videoDuration": f"PT{int(meetings.audio_seconds * 0.7)}S",
                        "screenShareDuration": f"PT{int(meetings.audio_seconds * 0.2)}S",
                    },
                )
            )
    return bulk_insert(client, "bronze_m365", "teams_activity", cols, rows)


def seed_m365_email_activity(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "bronze_m365", "email_activity")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "reportRefreshDate",
        "reportPeriod",
        "userPrincipalName",
        "displayName",
        "isDeleted",
        "lastActivityDate",
        "assignedProducts",
        "sendCount",
        "receiveCount",
        "readCount",
        "meetingCreatedCount",
        "meetingInteractedCount",
    ]
    rows: list[tuple[object, ...]] = []
    for person in _collab_persons(roster):
        for day in days_window(days):
            email = _email(person, day, "m365")
            if email.sent == 0 and email.received == 0:
                continue
            key = f"{tenant_uuid}:{M365_SOURCE_ID}:mail:{person.email}:{day.isoformat()}"
            rows.append(
                _positioned(
                    cols,
                    {
                        **_envelope(M365_SOURCE_ID, tenant_uuid),
                        "_airbyte_raw_id": deterministic_uuid("collab.m365.mail.raw", key),
                        "unique_key": key,
                        "reportRefreshDate": day.isoformat(),
                        "reportPeriod": "1",
                        "userPrincipalName": person.email,
                        "displayName": person.display_name,
                        "isDeleted": False,
                        "lastActivityDate": day.isoformat(),
                        "assignedProducts": '["EXCHANGE"]',
                        "sendCount": email.sent,
                        "receiveCount": email.received,
                        "readCount": email.read,
                        "meetingCreatedCount": 0,
                        "meetingInteractedCount": 0,
                    },
                )
            )
    return bulk_insert(client, "bronze_m365", "email_activity", cols, rows)


def seed_slack_users_details(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "bronze_slack", "users_details")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "date",
        "team_id",
        "user_id",
        "email_address",
        "is_active",
        "is_billable_seat",
        "is_guest",
        "messages_posted_count",
        "channel_messages_posted_count",
        "reactions_added_count",
        "files_added_count",
    ]
    rows: list[tuple[object, ...]] = []
    for person in _collab_persons(roster):
        for day in days_window(days):
            chat = _chat(person, day, "slack")
            if chat.total == 0:
                continue
            key = f"{tenant_uuid}:{SLACK_SOURCE_ID}:{person.email}:{day.isoformat()}"
            rows.append(
                _positioned(
                    cols,
                    {
                        **_envelope(SLACK_SOURCE_ID, tenant_uuid),
                        "_airbyte_raw_id": deterministic_uuid("collab.slack.raw", key),
                        "unique_key": key,
                        "date": day.isoformat(),
                        "team_id": "T000DEMO",
                        "user_id": person.uuid,
                        "email_address": person.email,
                        "is_active": True,
                        "is_billable_seat": True,
                        "is_guest": False,
                        "messages_posted_count": chat.total,
                        "channel_messages_posted_count": chat.channel,
                        "reactions_added_count": 0,
                        "files_added_count": 0,
                    },
                )
            )
    inserted = bulk_insert(client, "bronze_slack", "users_details", cols, rows)
    prune_to_bronze(client, ("staging", "slack__users_snapshot"), tenant_uuid)
    return inserted


def seed_zoom_meetings(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    """Meetings and who was in them.

    `zoom__collab_meeting_activity` derives attendance and duration from the
    participants' join/leave spans, so a meeting with no participant row is a
    meeting nobody attended.
    """
    truncate(client, "bronze_zoom", "meetings")
    truncate(client, "bronze_zoom", "participants")
    meeting_cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "uuid",
        "id",
        "topic",
        "host",
        "email",
        "start_time",
        "end_time",
        "duration",
        "participants",
    ]
    participant_cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "meeting_uuid",
        "id",
        "participant_uuid",
        "user_id",
        "user_name",
        "email",
        "join_time",
        "leave_time",
        "share_application",
        "share_desktop",
        "share_whiteboard",
    ]
    meetings: list[tuple[object, ...]] = []
    participants: list[tuple[object, ...]] = []

    for person in _collab_persons(roster):
        for day in days_window(days):
            drawn = _meetings(person, day, "zoom")
            if drawn.count == 0:
                continue
            # The day's total attendance split evenly across its meetings, and
            # the person themself in every one of them.
            per_meeting = max(1, drawn.participants // drawn.count)
            seconds_each = int(drawn.audio_seconds // drawn.count)
            for index in range(drawn.count):
                uuid = deterministic_uuid(
                    "collab.zoom.meeting", person.uuid, day.isoformat(), str(index)
                )
                starts = _dt.datetime.combine(day, _dt.time(9, tzinfo=_dt.UTC)) + _dt.timedelta(
                    hours=index
                )
                ends = starts + _dt.timedelta(seconds=seconds_each)
                meetings.append(
                    _positioned(
                        meeting_cols,
                        {
                            **_envelope(ZOOM_SOURCE_ID, tenant_uuid),
                            "_airbyte_raw_id": deterministic_uuid("collab.zoom.meeting.raw", uuid),
                            "unique_key": f"{tenant_uuid}:{ZOOM_SOURCE_ID}:{uuid}",
                            "uuid": uuid,
                            "id": deterministic_int("collab.zoom.meeting.id", uuid),
                            "topic": "Team sync",
                            "host": person.uuid,
                            "email": person.email,
                            "start_time": starts.isoformat(),
                            "end_time": ends.isoformat(),
                            # Zoom reports the minute count as text.
                            "duration": str(max(1, seconds_each // 60)),
                            "participants": per_meeting,
                        },
                    )
                )
                for seat in range(per_meeting):
                    # Seat 0 is the person whose day this is. The rest carry NO
                    # email: the staging model reads an emailless participant as
                    # `guest:<uuid>`, so they give the meeting the company
                    # presence it needs to count without becoming people of
                    # their own. A meeting whose only attendee is alone
                    # throughout earns no company time and is not attended.
                    attendee_email = person.email if seat == 0 else ""
                    attendee = deterministic_uuid("collab.zoom.participant", uuid, str(seat))
                    participants.append(
                        _positioned(
                            participant_cols,
                            {
                                **_envelope(ZOOM_SOURCE_ID, tenant_uuid),
                                "_airbyte_raw_id": deterministic_uuid(
                                    "collab.zoom.participant.raw", attendee
                                ),
                                "unique_key": f"{tenant_uuid}:{ZOOM_SOURCE_ID}:{attendee}",
                                "meeting_uuid": uuid,
                                "id": attendee,
                                "participant_uuid": attendee,
                                "user_id": attendee,
                                "user_name": attendee_email or f"Guest {seat + 1}",
                                "email": attendee_email,
                                "join_time": starts.isoformat(),
                                "leave_time": ends.isoformat(),
                                "share_application": False,
                                "share_desktop": seat == 0,
                                "share_whiteboard": False,
                            },
                        )
                    )

    return {
        "bronze_zoom.meetings": bulk_insert(
            client, "bronze_zoom", "meetings", meeting_cols, meetings
        ),
        "bronze_zoom.participants": bulk_insert(
            client, "bronze_zoom", "participants", participant_cols, participants
        ),
    }


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    truncate(client, "staging", "m365__collab_chat_activity")
    truncate(client, "staging", "m365__collab_email_activity")
    truncate(client, "staging", "m365__collab_meeting_activity")
    truncate(client, "staging", "slack__collab_chat_activity")
    truncate(client, "staging", "zoom__collab_meeting_activity")
    truncate(client, "staging", "zoom__meeting_sessions")
    truncate(client, "silver", "class_collab_chat_activity")
    truncate(client, "silver", "class_collab_email_activity")
    truncate(client, "silver", "class_collab_meeting_activity")

    return {
        "bronze_m365.teams_activity": seed_m365_teams_activity(client, roster, tenant_uuid, days),
        "bronze_m365.email_activity": seed_m365_email_activity(client, roster, tenant_uuid, days),
        "bronze_slack.users_details": seed_slack_users_details(client, roster, tenant_uuid, days),
        **seed_zoom_meetings(client, roster, tenant_uuid, days),
    }
