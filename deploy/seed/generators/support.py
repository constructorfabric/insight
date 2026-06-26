"""
Support silver-table generator: per-person-per-day ticket activity.

Only the support team produces rows here; other teams have zero
`zendesk-placeholder` weight in their TEAM_PROFILES entry. The
columns mirror what the
`20260611000000_support-bullet-rows.sql` gold migration reads from
`silver.class_support_activity` (updates / comments / solved / CSAT).
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


# Single data_source for now — there's no real Zendesk connector in the
# repo (see profiles.py support TeamProfile). The migration is
# data_source-agnostic; this string just lets the UI distinguish rows.
SOURCE = "zendesk-placeholder"


def _eligible(roster: Sequence[Person]) -> list[Person]:
    """Support-team members with a non-zero zendesk-placeholder weight."""
    return [
        p for p in roster
        if p.team and TEAM_PROFILES[p.team].weights.get(SOURCE, 0) > 0
    ]


def seed_class_support_activity(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_support_activity")
    cols = [
        "insight_tenant_id", "person_key", "date", "data_source",
        "updates", "public_comments", "private_comments", "solved",
        "csat_good", "csat_total", "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in _eligible(roster):
        persona = persona_multiplier(p.uuid)
        weight = TEAM_PROFILES[p.team or ""].weights[SOURCE]
        for d in days_window(days):
            rng = seeded_rng(p.uuid, d, "support.activity")
            wkd = weekday_multiplier(d)
            updates = min(poisson(rng, 2.0 * weight * persona * wkd), 30)
            pub_comm = min(poisson(rng, 3.0 * weight * persona * wkd), 40)
            prv_comm = min(poisson(rng, 1.0 * weight * persona * wkd), 15)
            solved = min(poisson(rng, 0.8 * weight * persona * wkd), 10)
            # ~30% of solved tickets get a CSAT; ~75% of those are "good".
            csat_total = min(poisson(rng, max(solved * 0.3, 0)), 8)
            csat_good = sum(1 for _ in range(csat_total) if rng.random() < 0.75)
            if (updates + pub_comm + prv_comm + solved + csat_total) == 0:
                continue
            rows.append((
                tenant_uuid, p.email, d, SOURCE,
                float(updates), float(pub_comm), float(prv_comm), float(solved),
                float(csat_good), float(csat_total), version,
            ))
    return bulk_insert(client, "silver", "class_support_activity", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    return {
        "silver.class_support_activity": seed_class_support_activity(
            client, roster, tenant_uuid, days,
        ),
    }
