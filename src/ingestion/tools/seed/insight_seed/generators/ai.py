"""
AI tooling silver-table generator: dev usage + assistant usage + seat overage.

dev-usage (`silver.class_ai_dev_usage`) covers Cursor + Claude Code.
assistant-usage (`silver.class_ai_assistant_usage`) covers ChatGPT +
Claude web. The gold-view filters discriminate by `tool` and `surface`
so we honour those exact strings.
seat-overage (`silver.class_ai_overage`) covers Claude Team seat spend
against the ceiling an administrator set on it, at seat-month grain.
"""

from __future__ import annotations

import datetime as _dt
import json
from collections.abc import Sequence
from typing import TYPE_CHECKING

from ..profiles import TEAM_PROFILES, Person
from .base import (
    anchor_datetime,
    bulk_insert,
    days_window,
    deterministic_uuid,
    persona_multiplier,
    poisson,
    seeded_rng,
    truncate,
    weekday_multiplier,
)

if TYPE_CHECKING:
    import clickhouse_connect.driver.client


_DEV_TOOLS = (
    # (tool string, source_type key in profile weights)
    # tool_label derives in gold now (macros/ai_labels.sql), not seeded.
    ("cursor", "cursor"),
    ("claude_code", "claude_team"),
)
_ASSISTANT_TOOLS = (
    # (tool string, surface, source_type key in profile weights)
    # surface must be a canonical value the gold model filters on
    # (ai_metric_observations gates chat_assistant_conversations on
    # surface = 'chat'); 'web' is not in the surface enum.
    # tool_label/surface_label derive in gold now, not seeded.
    ("chatgpt", "chat", "chatgpt"),
    ("claude", "chat", "claude_team"),
)
# Which seats an administrator put on a tier. A seat left `unassigned` carries
# no ceiling, so the utilisation metric emits no row for it — the honest-NULL
# the gold pair relies on, which is only present in seeded data while some team
# stays unassigned. A team absent here is unassigned too.
_SEAT_TIER_BY_TEAM = {"development": "team_tier_1", "support": "unassigned"}
# The ceiling a tiered seat carries, in the cents the vendor reports: $100.00.
_SEAT_CEILING_CENTS = 10_000


def seed_ai_dev_usage(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_ai_dev_usage")
    cols = [
        "insight_tenant_id",
        "email",
        "day",
        "tool",
        "is_active",
        "agent_sessions",
        "chat_requests",
        "tool_use_offered",
        "tool_use_accepted",
        "lines_added",
        "lines_removed",
        "total_lines_added",
        "total_lines_removed",
        "accepted_lines_added",
        "spec_lines",
        "session_count",
        "total_chat_messages",
        "cost_cents",
        "commits_count",
        "pull_requests_count",
        "prs_with_cc_count",
        "prs_total_count",
        "conversation_count",
        "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in roster:
        if not p.team:
            continue
        profile = TEAM_PROFILES[p.team]
        persona = persona_multiplier(p.uuid)
        for tool, src_key in _DEV_TOOLS:
            weight = profile.weights.get(src_key, 0)
            if weight <= 0:
                continue
            for d in days_window(days):
                rng = seeded_rng(p.uuid, d, f"ai.dev.{tool}")
                base = 6 * persona * weight * weekday_multiplier(d)
                sessions = min(poisson(rng, base), 30)
                if sessions == 0:
                    continue
                offered = sessions * rng.randint(3, 8)
                accepted = int(offered * rng.uniform(0.4, 0.85))
                lines_add = min(int(accepted * rng.randint(3, 18)), 400)
                lines_rem = int(lines_add * rng.uniform(0.2, 0.8))
                cost = float(sessions) * rng.uniform(2.0, 12.0)
                rows.append(
                    (
                        tenant_uuid,
                        p.email,
                        d,
                        tool,
                        1,
                        float(sessions),
                        float(sessions * rng.randint(2, 6)),
                        float(offered),
                        float(accepted),
                        float(lines_add),
                        float(lines_rem),
                        float(lines_add),
                        float(lines_rem),
                        float(lines_add),
                        0.0,
                        float(sessions),
                        float(sessions * 4),
                        round(cost, 2),
                        float(rng.randint(0, 4)),
                        float(rng.randint(0, 3)),
                        float(rng.randint(0, 2)),
                        float(rng.randint(0, 4)),
                        float(sessions),
                        version,
                    )
                )
    return bulk_insert(client, "silver", "class_ai_dev_usage", cols, rows)


def seed_ai_assistant_usage(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_ai_assistant_usage")
    cols = [
        "insight_tenant_id",
        "source_id",
        "unique_key",
        "email",
        "day",
        "tool",
        "surface",
        "session_count",
        "conversation_count",
        "message_count",
        "action_count",
        "files_uploaded_count",
        "artifacts_created_count",
        "projects_created_count",
        "projects_used_count",
        "skills_used_count",
        "connectors_used_count",
        "thinking_message_count",
        "dispatch_turn_count",
        "search_count",
        "cost_cents",
        "surface_metrics_json",
        "source",
        "data_source",
        "collected_at",
        "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    now = anchor_datetime()
    for p in roster:
        if not p.team:
            continue
        profile = TEAM_PROFILES[p.team]
        persona = persona_multiplier(p.uuid)
        for tool, surface, src_key in _ASSISTANT_TOOLS:
            weight = profile.weights.get(src_key, 0)
            if weight <= 0:
                continue
            for d in days_window(days):
                rng = seeded_rng(p.uuid, d, f"ai.assistant.{tool}")
                base = 3 * persona * weight * weekday_multiplier(d)
                sessions = min(poisson(rng, base), 20)
                if sessions == 0:
                    continue
                msgs = sessions * rng.randint(4, 14)
                conversations = max(1, int(sessions * rng.uniform(0.6, 1.0)))
                rows.append(
                    (
                        tenant_uuid,
                        deterministic_uuid("ai.assistant.src", p.uuid, tool),
                        deterministic_uuid("ai.assistant.row", p.uuid, d.isoformat(), tool),
                        p.email,
                        d,
                        tool,
                        surface,
                        sessions,
                        conversations,
                        msgs,
                        sessions * 2,
                        rng.randint(0, 3),
                        rng.randint(0, 2),
                        rng.randint(0, 1),
                        rng.randint(0, 3),
                        rng.randint(0, 4),
                        rng.randint(0, 2),
                        sessions,
                        sessions,
                        msgs // 3,
                        int(sessions * rng.uniform(3.0, 9.0)),
                        None,
                        tool,
                        tool,
                        now,
                        version,
                    )
                )
    return bulk_insert(client, "silver", "class_ai_assistant_usage", cols, rows)


def _seat_month_reads(days: int) -> list[tuple[_dt.date, _dt.date]]:
    """One (billing month, day that month was last read) pair per month covered.

    The vendor reports spend-to-date for the month in progress and carries no
    period field, so a month's row freezes at its final read. That day is what
    dates the evidence, which is why the current month reads on the anchor.
    """
    last_read: dict[_dt.date, _dt.date] = {}
    for d in days_window(days):
        last_read[d.replace(day=1)] = d
    return sorted(last_read.items())


def seed_ai_seat_overage(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    """Claude Team seat spend against its ceiling, one row per seat per month.

    `used_amount_cents` is the money billed once a seat exhausted the usage its
    fee included — not the excess over the ceiling. A fifth of tiered
    seat-months land a few cents past the ceiling, which is what an enforced
    limit looks like.
    """
    truncate(client, "silver", "class_ai_overage")
    cols = [
        "insight_tenant_id",
        "source_id",
        "unique_key",
        "email",
        "account_id",
        "period_month",
        "tool",
        "seat_tier",
        "currency",
        "credit_limit_cents",
        "used_amount_cents",
        "overage_cents",
        "is_over_limit",
        "is_enabled",
        "overage_metrics_json",
        "source",
        "data_source",
        "collected_at",
        "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    # One connector instance serves the whole stream, as a tenant's single
    # claude-team source does.
    source_id = deterministic_uuid("ai.overage.src", tenant_uuid)
    for p in roster:
        if not p.team:
            continue
        if TEAM_PROFILES[p.team].weights.get("claude_team", 0) <= 0:
            continue
        persona = persona_multiplier(p.uuid)
        account_id = deterministic_uuid("ai.overage.seat", p.uuid)
        tier = _SEAT_TIER_BY_TEAM.get(p.team, "unassigned")
        ceiling = None if tier == "unassigned" else _SEAT_CEILING_CENTS
        for period_month, read_day in _seat_month_reads(days):
            rng = seeded_rng(p.uuid, read_day, "ai.overage")
            if ceiling is None:
                used = int(rng.uniform(50, 4000) * persona)
            elif rng.random() < 0.2:
                used = ceiling + rng.randint(2, 48)
            else:
                used = int(min(rng.uniform(300, 9000) * persona, ceiling - 100))
            rows.append(
                (
                    tenant_uuid,
                    source_id,
                    f"{tenant_uuid}-{source_id}-{account_id}-{period_month:%Y-%m}",
                    p.email,
                    account_id,
                    period_month,
                    "claude",
                    tier,
                    "USD",
                    ceiling,
                    used,
                    None if ceiling is None else max(0, used - ceiling),
                    None if ceiling is None else int(used > ceiling),
                    1,
                    json.dumps(
                        {
                            "limit_type": "" if ceiling is None else "seat_tier",
                            "used_credits_basis": "post_discount",
                            "out_of_credits": "",
                            "seat_tier": tier,
                        }
                    ),
                    "claude_team",
                    "insight_claude_team",
                    _dt.datetime.combine(read_day, _dt.time(), tzinfo=_dt.UTC),
                    version,
                )
            )
    return bulk_insert(client, "silver", "class_ai_overage", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    return {
        "silver.class_ai_dev_usage": seed_ai_dev_usage(client, roster, tenant_uuid, days),
        "silver.class_ai_assistant_usage": seed_ai_assistant_usage(
            client, roster, tenant_uuid, days
        ),
        "silver.class_ai_overage": seed_ai_seat_overage(client, roster, tenant_uuid, days),
    }
