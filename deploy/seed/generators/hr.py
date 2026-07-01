"""
HR / focus metrics generator.

Produces one row per person per day in `silver.class_focus_metrics` —
the table that drives `insight.ic_kpis.focus_time_pct` and `dev_time_h`.

`working_hours_per_day` is the headline number HR consumers see;
`meeting_hours` matches what the collab.meeting generator produces;
`focus_time_pct` = (working - meeting) / working, clamped sensibly.
"""

from __future__ import annotations

from collections.abc import Sequence
from typing import TYPE_CHECKING

from generators.base import (
    bulk_insert,
    clamp,
    days_window,
    deterministic_uuid,
    persona_multiplier,
    seeded_rng,
    truncate,
    weekday_multiplier,
)
from profiles import Person

if TYPE_CHECKING:
    import clickhouse_connect.driver.client


def _focus_persons(roster: Sequence[Person]) -> list[Person]:
    """Every non-CEO person has HR metrics."""
    return [p for p in roster if p.team is not None]


def seed_focus_metrics(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_focus_metrics")
    cols = [
        "insight_tenant_id", "email", "day", "unique_key",
        "meetings_count", "meeting_hours", "working_hours_per_day",
        "focus_time_pct", "dev_time_h", "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in _focus_persons(roster):
        persona = persona_multiplier(p.uuid)
        for d in days_window(days):
            rng = seeded_rng(p.uuid, d, "hr.focus")
            weekend = d.weekday() >= 5
            # Baseline 7h ± 1h on weekdays, 0-2h on weekends.
            if weekend:
                wh = clamp(rng.uniform(0, 2), 0, 12) * persona
            else:
                wh = clamp(rng.gauss(7.5 * persona, 1.0), 4, 12)
            # Meetings consume some of those hours — bounded at 6h on a
            # heavy day, generally less.
            m_hours = clamp(
                rng.gauss(1.5 * persona * weekday_multiplier(d), 0.8),
                0, min(wh * 0.7, 6),
            )
            m_count = round(m_hours * 1.5)
            focus_pct = 0.0 if wh < 1 else clamp((wh - m_hours) / wh * 100, 0, 100)
            dev_h = clamp(wh - m_hours, 0, 10)
            rows.append((
                tenant_uuid, p.email, d,
                deterministic_uuid("focus", p.uuid, d.isoformat()),
                m_count, round(m_hours, 2), round(wh, 2),
                round(focus_pct, 2), round(dev_h, 2), version,
            ))
    return bulk_insert(client, "silver", "class_focus_metrics", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    return {
        "silver.class_focus_metrics": seed_focus_metrics(client, roster, tenant_uuid, days),
    }
