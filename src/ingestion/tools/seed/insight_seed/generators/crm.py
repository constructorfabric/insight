"""CRM activity, as HubSpot reports it, into bronze.

Owners, deals and the three engagement streams the sales team works through;
`hubspot__crm_users`, `__crm_deals` and `__crm_activities` shape them into the
class relations. Only the sales team carries a hubspot weight, so every other
team produces nothing here.

The draws are unchanged — same salts, same order, same caps — so the deals and
engagements a run produces are the ones it produced before. Their SHAPE differs
where the connector's own vocabulary differs: an engagement's type comes out of
the stream it arrived on (`call` / `email` / `meeting`), not the upper-case
marker the generator used to write.
"""

from __future__ import annotations

import datetime as _dt
from collections.abc import Sequence
from typing import TYPE_CHECKING

from ..profiles import TEAM_PROFILES, Person
from .base import (
    anchor_date,
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
    import clickhouse_connect.driver.client


SOURCE_ID = deterministic_uuid("crm.source", "hubspot")
DATA_SOURCE = "insight_hubspot"

AIRBYTE_COLS = [
    "_airbyte_raw_id",
    "_airbyte_extracted_at",
    "_airbyte_meta",
    "_airbyte_generation_id",
]
IDENTITY_COLS = ["unique_key", "tenant_id", "source_id", "data_source", "collected_at"]

#: Engagement kind -> the bronze stream HubSpot delivers it on. The cycle order
#: is the generator's own and unchanged.
_CRM_ACTIVITY_STREAMS = ("engagements_calls", "engagements_emails", "engagements_meetings")


def _eligible(roster: Sequence[Person]) -> list[Person]:
    """Persons whose team profile has any hubspot weight."""
    return [p for p in roster if p.team and TEAM_PROFILES[p.team].weights.get("hubspot", 0) > 0]


def _owner_id(p: Person) -> str:
    """Stable per-person CRM owner id."""
    return deterministic_uuid("crm.owner", p.uuid)


def _envelope(tenant_uuid: str) -> dict[str, object]:
    read_at = anchor_datetime()
    return {
        "_airbyte_extracted_at": read_at,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": tenant_uuid,
        "source_id": SOURCE_ID,
        "data_source": DATA_SOURCE,
        "collected_at": read_at.isoformat(),
    }


def _positioned(cols: list[str], fields: dict[str, object]) -> tuple[object, ...]:
    return tuple(fields.get(col) for col in cols)


def seed_owners(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
) -> int:
    truncate(client, "bronze_hubspot", "owners")
    cols = [
        *AIRBYTE_COLS,
        *IDENTITY_COLS,
        "id",
        "email",
        "firstName",
        "lastName",
        "userId",
        "createdAt",
        "updatedAt",
        "archived",
        "archivedAt",
    ]
    at = anchor_datetime()
    rows = [
        _positioned(
            cols,
            {
                **_envelope(tenant_uuid),
                "_airbyte_raw_id": deterministic_uuid("crm.owner.raw", person.uuid),
                "unique_key": f"{tenant_uuid}:{SOURCE_ID}:{_owner_id(person)}",
                "id": _owner_id(person),
                "email": person.email,
                "firstName": person.first_name,
                "lastName": person.last_name,
                # HubSpot's own user id is numeric, and the stream types it so.
                "userId": deterministic_int("crm.hsuser", person.uuid),
                "createdAt": at,
                "updatedAt": at,
                "archived": False,
                "archivedAt": None,
            },
        )
        for person in _eligible(roster)
    ]
    return bulk_insert(client, "bronze_hubspot", "owners", cols, rows)


def seed_deals(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "bronze_hubspot", "deals")
    cols = [
        *AIRBYTE_COLS,
        *IDENTITY_COLS,
        "id",
        "createdAt",
        "updatedAt",
        "archived",
        "properties_dealname",
        "properties_dealstage",
        "properties_amount",
        "properties_amount_in_home_currency",
        "properties_closedate",
        "properties_hubspot_owner_id",
        "properties_hs_created_by_user_id",
        "properties_hs_is_closed",
        "properties_hs_is_closed_won",
    ]
    rows: list[tuple[object, ...]] = []
    for person in _eligible(roster):
        persona = persona_multiplier(person.uuid)
        weight = TEAM_PROFILES[person.team or ""].weights["hubspot"]
        owner = _owner_id(person)
        for day in days_window(days):
            rng = seeded_rng(person.uuid, day, "crm.deals")
            mean = 0.6 * persona * weight * weekday_multiplier(day)
            for index in range(min(poisson(rng, mean), 8)):
                deal_id = deterministic_uuid("crm.deal", person.uuid, day.isoformat(), str(index))
                created_dt = _dt.datetime.combine(
                    day, _dt.time(9 + rng.randint(0, 8), rng.randint(0, 59), tzinfo=_dt.UTC)
                )
                close = day + _dt.timedelta(days=rng.randint(7, 60))
                is_closed = close < anchor_date()
                is_won = is_closed and rng.random() < 0.35
                amount = round(rng.uniform(500, 50000), 2)
                rows.append(
                    _positioned(
                        cols,
                        {
                            **_envelope(tenant_uuid),
                            "_airbyte_raw_id": deterministic_uuid("crm.deal.raw", deal_id),
                            "unique_key": f"{tenant_uuid}:{SOURCE_ID}:{deal_id}",
                            "id": deal_id,
                            "createdAt": created_dt,
                            "updatedAt": created_dt,
                            "archived": False,
                            "properties_dealname": f"Deal {deal_id[:8]}",
                            "properties_dealstage": "closedwon" if is_won else "qualifiedtobuy",
                            "properties_amount": f"{amount}",
                            "properties_amount_in_home_currency": f"{amount}",
                            "properties_closedate": close.isoformat(),
                            "properties_hubspot_owner_id": owner,
                            "properties_hs_created_by_user_id": owner,
                            "properties_hs_is_closed": "true" if is_closed else "false",
                            "properties_hs_is_closed_won": "true" if is_won else "false",
                        },
                    )
                )
    return bulk_insert(client, "bronze_hubspot", "deals", cols, rows)


def seed_engagements(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    """Calls, emails and meetings, each on the stream HubSpot delivers it on."""
    shared = [
        "id",
        "createdAt",
        "updatedAt",
        "archived",
        "properties_hs_timestamp",
        "properties_hubspot_owner_id",
        "properties_hs_created_by_user_id",
        "associations_contacts",
        "associations_companies",
        "associations_deals",
    ]
    #: The one column per stream the staging model reads for its own kind.
    specific = {
        "engagements_calls": {"properties_hs_call_title": "Discovery call"},
        "engagements_emails": {"properties_hs_email_subject": "Follow-up"},
        "engagements_meetings": {"properties_hs_meeting_title": "Pipeline review"},
    }
    cols = {
        stream: [*AIRBYTE_COLS, *IDENTITY_COLS, *shared, *extra]
        for stream, extra in specific.items()
    }
    rows: dict[str, list[tuple[object, ...]]] = {stream: [] for stream in specific}
    # Spelled out rather than looped: the reset registry is greppable by
    # contract, and test_preflight scans these call sites for it.
    truncate(client, "bronze_hubspot", "engagements_calls")
    truncate(client, "bronze_hubspot", "engagements_emails")
    truncate(client, "bronze_hubspot", "engagements_meetings")

    for person in _eligible(roster):
        persona = persona_multiplier(person.uuid)
        weight = TEAM_PROFILES[person.team or ""].weights["hubspot"]
        owner = _owner_id(person)
        for day in days_window(days):
            rng = seeded_rng(person.uuid, day, "crm.activities")
            mean = 6 * persona * weight * weekday_multiplier(day)
            for index in range(min(poisson(rng, mean), 20)):
                stream = _CRM_ACTIVITY_STREAMS[index % len(_CRM_ACTIVITY_STREAMS)]
                at = _dt.datetime.combine(
                    day, _dt.time(9 + rng.randint(0, 8), rng.randint(0, 59), tzinfo=_dt.UTC)
                )
                activity_id = deterministic_uuid(
                    "crm.activity", person.uuid, day.isoformat(), str(index)
                )
                rows[stream].append(
                    _positioned(
                        cols[stream],
                        {
                            **_envelope(tenant_uuid),
                            **specific[stream],
                            "_airbyte_raw_id": deterministic_uuid("crm.activity.raw", activity_id),
                            "unique_key": f"{tenant_uuid}:{SOURCE_ID}:{activity_id}",
                            "id": activity_id,
                            "createdAt": at,
                            "updatedAt": at,
                            "archived": False,
                            "properties_hs_timestamp": at.isoformat(),
                            "properties_hubspot_owner_id": owner,
                            "properties_hs_created_by_user_id": owner,
                            "associations_contacts": "[]",
                            "associations_companies": "[]",
                            "associations_deals": "[]",
                        },
                    )
                )

    return {
        f"bronze_hubspot.{stream}": bulk_insert(
            client, "bronze_hubspot", stream, cols[stream], rows[stream]
        )
        for stream in specific
    }


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    # The staging models append rather than replace, so a re-seed at another
    # anchor would stack a second copy of the window behind the first.
    truncate(client, "staging", "hubspot__crm_users")
    truncate(client, "staging", "hubspot__crm_deals")
    truncate(client, "staging", "hubspot__crm_activities")
    truncate(client, "silver", "class_crm_users")
    truncate(client, "silver", "class_crm_deals")
    truncate(client, "silver", "class_crm_activities")

    return {
        "bronze_hubspot.owners": seed_owners(client, roster, tenant_uuid),
        "bronze_hubspot.deals": seed_deals(client, roster, tenant_uuid, days),
        **seed_engagements(client, roster, tenant_uuid, days),
    }
