"""
AI tooling silver-table generator: dev usage + assistant usage.

dev-usage (`silver.class_ai_dev_usage`) covers Cursor + Claude Code.
assistant-usage (`silver.class_ai_assistant_usage`) covers ChatGPT +
Claude web. The gold-view filters discriminate by `tool` and `surface`
so we honour those exact strings.
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
    now = _dt.datetime.now(_dt.UTC).replace(tzinfo=None)
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
    }
