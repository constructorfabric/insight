"""Support activity, as Zendesk reports it, into bronze.

The per-person-per-day counts silver serves are derived by
`zendesk__support_activity` from audits and ratings, so this generator emits
those — one audit per update, per comment and per solve, one rating per CSAT
response — rather than the totals. The draws are unchanged: what a person does
on a day is still decided here, only the shape it is written in moved.

Attribution runs through `bronze_zendesk.support_agents`: the staging model
joins an audit's `author_id` to an agent and takes `lower(email)` as the person
key, so a person with no agent row absorbs nothing.
"""

from __future__ import annotations

import json
from collections.abc import Iterator, Sequence
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

if TYPE_CHECKING:
    import datetime as _dt

    import clickhouse_connect.driver.client


ZENDESK_SOURCE_ID = deterministic_uuid("support.source", "zendesk")

#: The TEAM_PROFILES key carrying a team's support volume. Still spelled
#: `-placeholder` because `task.py` keys its own support volume off the same
#: entry; renaming it belongs with that generator's own move onto bronze.
WEIGHT_KEY = "zendesk-placeholder"


class DayVolumes(NamedTuple):
    """What one person did on one day, before it is written as events."""

    updates: int
    public_comments: int
    private_comments: int
    solved: int
    csat_good: int
    csat_total: int


def _eligible(roster: Sequence[Person]) -> list[Person]:
    """Support-team members with a non-zero support weight."""
    return [p for p in roster if p.team and TEAM_PROFILES[p.team].weights.get(WEIGHT_KEY, 0) > 0]


def _volumes(person: Person, day: _dt.date) -> DayVolumes:
    """The day's activity, drawn exactly as it was before this generator
    wrote bronze — same salt, same order — so a stand's numbers did not move."""
    rng = seeded_rng(person.uuid, day, "support.activity")
    scale = (
        TEAM_PROFILES[person.team or ""].weights[WEIGHT_KEY]
        * persona_multiplier(person.uuid)
        * weekday_multiplier(day)
    )

    # INVARIANT: `poisson` consumes rng state, so the ORDER of these draws is
    # part of the dataset. Reordering them reseeds every support row.
    updates = min(poisson(rng, 2.0 * scale), 30)
    public_comments = min(poisson(rng, 3.0 * scale), 40)
    private_comments = min(poisson(rng, 1.0 * scale), 15)
    solved = min(poisson(rng, 0.8 * scale), 10)
    # ~30% of solved tickets get a CSAT; ~75% of those are "good".
    csat_total = min(poisson(rng, max(solved * 0.3, 0)), 8)
    csat_good = sum(1 for _ in range(csat_total) if rng.random() < 0.75)

    return DayVolumes(updates, public_comments, private_comments, solved, csat_good, csat_total)


def _envelope(tenant_uuid: str) -> dict[str, object]:
    read_at = anchor_datetime()
    return {
        "_airbyte_extracted_at": read_at,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": tenant_uuid,
        "source_id": ZENDESK_SOURCE_ID,
        "collected_at": read_at.isoformat(),
        "data_source": "zendesk",
    }


def _positioned(cols: list[str], fields: dict[str, object]) -> tuple[object, ...]:
    return tuple(fields.get(col) for col in cols)


AIRBYTE_COLS = [
    "_airbyte_raw_id",
    "_airbyte_extracted_at",
    "_airbyte_meta",
    "_airbyte_generation_id",
]


def _agent_id(person: Person) -> str:
    return str(deterministic_int("support.agent", person.email))


def seed_support_agents(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
) -> int:
    """The agent directory every audit and rating is attributed through."""
    truncate(client, "bronze_zendesk", "support_agents")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "data_source",
        "agent_id",
        "email",
        "display_name",
        "role",
        "group_id",
        "group_name",
        "is_active",
    ]
    envelope = _envelope(tenant_uuid)
    rows = [
        _positioned(
            cols,
            {
                **envelope,
                "_airbyte_raw_id": deterministic_uuid("support.agent.raw", person.email),
                "unique_key": f"{tenant_uuid}:{ZENDESK_SOURCE_ID}:{_agent_id(person)}",
                "agent_id": _agent_id(person),
                "email": person.email,
                "display_name": person.display_name,
                # The staging model maps this to `agent`; a lead is an agent in
                # Zendesk's own vocabulary, not an admin.
                "role": "agent",
                "group_id": "1",
                "group_name": "Support",
                "is_active": 1,
            },
        )
        for person in _eligible(roster)
    ]
    return bulk_insert(client, "bronze_zendesk", "support_agents", cols, rows)


class Audit(NamedTuple):
    audit_id: str
    ticket_id: str
    events: str


def _audits(person: Person, day: _dt.date, volumes: DayVolumes) -> Iterator[Audit]:
    """One audit per counted thing.

    The staging model counts DISTINCT audits for `updates` and DISTINCT tickets
    for `solved`, so those get an audit and a ticket of their own; comments are
    counted per event, so one each is enough.
    """

    def ident(kind: str, index: int) -> str:
        return str(
            deterministic_int("support.audit", person.uuid, day.isoformat(), kind, str(index))
        )

    def ticket(index: int) -> str:
        return str(deterministic_int("support.ticket", person.uuid, day.isoformat(), str(index)))

    for index in range(volumes.updates):
        audit = ident("update", index)
        # A Change on any field but status→solved classifies as `update`.
        yield Audit(
            audit,
            ticket(index),
            json.dumps(
                [{"id": audit, "type": "Change", "field_name": "priority", "value": "high"}]
            ),
        )

    for index in range(volumes.public_comments):
        audit = ident("public", index)
        yield Audit(
            audit, ticket(index), json.dumps([{"id": audit, "type": "Comment", "public": True}])
        )

    for index in range(volumes.private_comments):
        audit = ident("private", index)
        yield Audit(
            audit, ticket(index), json.dumps([{"id": audit, "type": "Comment", "public": False}])
        )

    for index in range(volumes.solved):
        audit = ident("solved", index)
        # A distinct ticket each: `solved` is uniqExact over the ticket, so two
        # solves of one ticket are one solved ticket.
        yield Audit(
            audit,
            ticket(1000 + index),
            json.dumps(
                [{"id": audit, "type": "Change", "field_name": "status", "value": "solved"}]
            ),
        )


def seed_support_ticket_events(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    """The ticket audits `updates`, the comment counts and `solved` come from."""
    truncate(client, "bronze_zendesk", "support_ticket_events")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "data_source",
        "audit_id",
        "ticket_id",
        "author_id",
        "created_at",
        "events",
    ]
    envelope = _envelope(tenant_uuid)
    rows: list[tuple[object, ...]] = []
    for person in _eligible(roster):
        author = _agent_id(person)
        for day in days_window(days):
            # Midday, so a row's date is the same one the draw was made for
            # whatever timezone a reader renders it in.
            occurred = f"{day.isoformat()}T12:00:00Z"
            for audit in _audits(person, day, _volumes(person, day)):
                rows.append(
                    _positioned(
                        cols,
                        {
                            **envelope,
                            "_airbyte_raw_id": deterministic_uuid(
                                "support.audit.raw", audit.audit_id
                            ),
                            "unique_key": f"{tenant_uuid}:{ZENDESK_SOURCE_ID}:{audit.audit_id}",
                            "audit_id": audit.audit_id,
                            "ticket_id": audit.ticket_id,
                            "author_id": author,
                            "created_at": occurred,
                            "events": audit.events,
                        },
                    )
                )
    return bulk_insert(client, "bronze_zendesk", "support_ticket_events", cols, rows)


def seed_satisfaction_ratings(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    """CSAT responses, bound to the ticket ASSIGNEE.

    The one measure here that is not actor-attributed: the customer sets the
    score, so the staging model credits whoever the ticket was assigned to.
    """
    truncate(client, "bronze_zendesk", "zendesk_satisfaction_ratings")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "data_source",
        "rating_id",
        "ticket_id",
        "requester_id",
        "assignee_id",
        "group_id",
        "score",
        "comment",
        "reason",
        "created_at",
        "updated_at",
    ]
    envelope = _envelope(tenant_uuid)
    rows: list[tuple[object, ...]] = []
    for person in _eligible(roster):
        assignee = _agent_id(person)
        for day in days_window(days):
            volumes = _volumes(person, day)
            rated = f"{day.isoformat()}T13:00:00Z"
            for index in range(volumes.csat_total):
                rating_id = str(
                    deterministic_int("support.rating", person.uuid, day.isoformat(), str(index))
                )
                rows.append(
                    _positioned(
                        cols,
                        {
                            **envelope,
                            "_airbyte_raw_id": deterministic_uuid("support.rating.raw", rating_id),
                            "unique_key": f"{tenant_uuid}:{ZENDESK_SOURCE_ID}:{rating_id}",
                            "rating_id": rating_id,
                            "ticket_id": str(
                                deterministic_int(
                                    "support.ticket",
                                    person.uuid,
                                    day.isoformat(),
                                    str(1000 + index),
                                )
                            ),
                            "requester_id": None,
                            "assignee_id": assignee,
                            "group_id": "1",
                            # `good` for the first csat_good of them: the draw
                            # decided how many, not which.
                            "score": "good" if index < volumes.csat_good else "bad",
                            "comment": None,
                            "reason": None,
                            "created_at": rated,
                            "updated_at": rated,
                        },
                    )
                )
    return bulk_insert(client, "bronze_zendesk", "zendesk_satisfaction_ratings", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    # The staging arms carry an `_airbyte_extracted_at` watermark and silver a
    # `_version` one, so rows a previous run left behind at another anchor are
    # never revisited and would stand forever beside these.
    truncate(client, "staging", "zendesk__support_agent")
    truncate(client, "staging", "zendesk__support_event")
    truncate(client, "staging", "zendesk__support_activity")
    truncate(client, "silver", "class_support_activity")

    return {
        "bronze_zendesk.support_agents": seed_support_agents(client, roster, tenant_uuid),
        "bronze_zendesk.support_ticket_events": seed_support_ticket_events(
            client, roster, tenant_uuid, days
        ),
        "bronze_zendesk.zendesk_satisfaction_ratings": seed_satisfaction_ratings(
            client, roster, tenant_uuid, days
        ),
    }
