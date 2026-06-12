"""
crm silver-table generator: users + deals + activities.

Only the sales team produces CRM activity. The dev/hr/support teams
get zero rows here.
"""

from __future__ import annotations

import datetime as _dt
from collections.abc import Sequence
from typing import TYPE_CHECKING

from generators.base import (
    bulk_insert,
    days_window,
    deterministic_uuid,
    persona_multiplier,
    poisson,
    seeded_rng,
    truncate,
    weekday_multiplier,
)
from profiles import TEAM_PROFILES, Person

if TYPE_CHECKING:
    import clickhouse_connect.driver.client


def _eligible(roster: Sequence[Person]) -> list[Person]:
    """Persons whose team profile has any hubspot weight."""
    return [
        p for p in roster
        if p.team and TEAM_PROFILES[p.team].weights.get("hubspot", 0) > 0
    ]


def _owner_id(p: Person) -> str:
    """Stable per-person CRM owner id."""
    return deterministic_uuid("crm.owner", p.uuid)


def seed_class_crm_users(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
) -> int:
    truncate(client, "silver", "class_crm_users")
    cols = ["unique_key", "user_id", "hs_user_id", "email", "_version"]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in _eligible(roster):
        owner = _owner_id(p)
        rows.append((
            deterministic_uuid("crm.user", p.email),
            owner, owner, p.email, version,
        ))
    return bulk_insert(client, "silver", "class_crm_users", cols, rows)


def seed_class_crm_deals(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    days: int,
) -> int:
    truncate(client, "silver", "class_crm_deals")
    cols = [
        "unique_key", "created_at", "close_date", "is_won", "is_closed",
        "amount_home", "owner_id", "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in _eligible(roster):
        persona = persona_multiplier(p.uuid)
        weight = TEAM_PROFILES[p.team or ""].weights["hubspot"]
        for d in days_window(days):
            rng = seeded_rng(p.uuid, d, "crm.deals")
            mean = 0.6 * persona * weight * weekday_multiplier(d)
            n_deals = min(poisson(rng, mean), 8)
            for i in range(n_deals):
                deal_id = deterministic_uuid("crm.deal", p.uuid, d.isoformat(), str(i))
                created_dt = _dt.datetime.combine(
                    d, _dt.time(9 + rng.randint(0, 8), rng.randint(0, 59)),
                )
                cycle = rng.randint(7, 60)
                close = d + _dt.timedelta(days=cycle)
                # Closed if the close date is in the past (i.e. before today).
                today = _dt.datetime.now(_dt.UTC).date()
                is_closed = 1 if close < today else 0
                is_won = 1 if is_closed and rng.random() < 0.35 else 0
                amount = round(rng.uniform(500, 50000), 2)
                rows.append((
                    deal_id, created_dt, close,
                    is_won, is_closed, amount, _owner_id(p), version,
                ))
    return bulk_insert(client, "silver", "class_crm_deals", cols, rows)


_CRM_ACTIVITY_TYPES = ("CALL", "EMAIL", "MEETING")


def seed_class_crm_activities(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    days: int,
) -> int:
    truncate(client, "silver", "class_crm_activities")
    cols = [
        "unique_key", "timestamp", "activity_type", "owner_id",
        "created_by_user_id", "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in _eligible(roster):
        persona = persona_multiplier(p.uuid)
        weight = TEAM_PROFILES[p.team or ""].weights["hubspot"]
        owner = _owner_id(p)
        for d in days_window(days):
            rng = seeded_rng(p.uuid, d, "crm.activities")
            mean = 6 * persona * weight * weekday_multiplier(d)
            n_acts = min(poisson(rng, mean), 20)
            for i in range(n_acts):
                kind = _CRM_ACTIVITY_TYPES[i % len(_CRM_ACTIVITY_TYPES)]
                ts = _dt.datetime.combine(
                    d, _dt.time(9 + rng.randint(0, 8), rng.randint(0, 59)),
                )
                rows.append((
                    deterministic_uuid("crm.activity", p.uuid, d.isoformat(), str(i)),
                    ts, kind, owner, owner, version,
                ))
    return bulk_insert(client, "silver", "class_crm_activities", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    return {
        "silver.class_crm_users":      seed_class_crm_users(client, roster),
        "silver.class_crm_deals":      seed_class_crm_deals(client, roster, days),
        "silver.class_crm_activities": seed_class_crm_activities(client, roster, days),
    }
