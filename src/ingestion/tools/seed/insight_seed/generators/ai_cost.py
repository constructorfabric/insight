"""
AI-cost generator: what a Claude Team seat spends, and what the vendor billed.

Distinct from `ai.py`, which seeds tool USE: these two carry money, at a
different grain (seat-month and invoice line, not day) and for a different
reader — the cost views, not the adoption ones.

seat-overage (`silver.class_ai_overage`) is seeded at silver. Its bronze key
carries no month, and the bronze table is ordered by that key, so three monthly
reads collapse into the freshest one and a stand built in a single pass would
hold one billing month instead of the window the metrics read.

invoices (`bronze_claude_team_invoices.claude_team_invoice_lines`) are seeded at
bronze instead: an invoice line has its own key, nothing collapses, and the
connector's own staging model then runs on the stand — the cents that must not
be multiplied, the per-line grain a seat price is recoverable from.
"""

from __future__ import annotations

import datetime as _dt
import json
from collections.abc import Sequence
from typing import TYPE_CHECKING

from ..profiles import TEAM_PROFILES, Person
from .base import (
    bulk_insert,
    days_window,
    deterministic_uuid,
    persona_multiplier,
    seeded_rng,
    truncate,
)

if TYPE_CHECKING:
    import clickhouse_connect.driver.client


# Which seats an administrator put on a tier. A seat left `unassigned` carries
# no ceiling, so the utilisation metric emits no row for it — the honest-NULL
# the gold pair relies on, which is only present in seeded data while some team
# stays unassigned. A team absent here is unassigned too.
_SEAT_TIER_BY_TEAM = {"development": "team_tier_1", "support": "unassigned"}
# The ceiling a tiered seat carries, in the cents the vendor reports: $100.00.
_SEAT_CEILING_CENTS = 10_000
# What the tier itself costs per month, in the same cents: $12.00. The
# invoice prices seats with it; the ceiling above bounds extra usage only.
_SEAT_PRICE_CENTS = 1_200


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


def seed_claude_team_invoices_bronze(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    """Vendor invoices as the Stripe chain returns them, into bronze.

    An invoice is an organisation-level fact, not a per-person one: one per
    billing month, priced per tier, plus the extra usage the same month billed.
    Its lines are what a seat price is recoverable from, so each invoice emits its
    own row carrying its money and one row per line, and aggregation stays gold's job.

    The layers above are cleared too: this is the only generator that seeds bronze
    and lets dbt build staging and silver, and both are incremental behind a strict
    `_airbyte_extracted_at >` watermark. A re-seed writes the same deterministic
    timestamps, so without this a shorter window would leave the months that dropped
    out of it standing in staging and silver.
    """
    truncate(client, "silver", "class_ai_invoice")
    truncate(client, "staging", "claude_team__ai_invoice")
    truncate(client, "bronze_claude_team_invoices", "claude_team_invoice_lines")
    cols = [
        "_airbyte_raw_id",
        "_airbyte_extracted_at",
        "_airbyte_meta",
        "_airbyte_generation_id",
        "tenant_id",
        "source_id",
        "unique_key",
        "collected_at",
        "data_source",
        "chain_status",
        "invoice_id",
        "invoice_status",
        "invoice_created_ts",
        "invoice_due_date_ts",
        "invoice_currency",
        "invoice_total",
        "invoice_total_excluding_tax",
        "invoice_num_seats",
        "invoice_payment_intent",
        "line_id",
        "description",
        "product_name",
        "tier_label",
        "category",
        "is_proration",
        "amount",
        "currency",
        "quantity",
        "unit_amount",
        "seat_unit_amount",
        "period_start_ts",
        "period_end_ts",
    ]

    def bronze_row(**fields: object) -> tuple[object, ...]:
        """Position `fields` against `cols`; anything unset is NULL, as the connector leaves it."""
        return tuple(fields.get(col) for col in cols)

    source_id = deterministic_uuid("ai.invoice.src", tenant_uuid)
    tiered_seats = sum(
        1
        for p in roster
        if p.team
        and TEAM_PROFILES[p.team].weights.get("claude_team", 0) > 0
        and _SEAT_TIER_BY_TEAM.get(p.team, "unassigned") != "unassigned"
    )
    rows: list[tuple[object, ...]] = []
    for period_month, read_day in _seat_month_reads(days):
        seats_total = tiered_seats * _SEAT_PRICE_CENTS
        extra = int(seats_total * 0.1)
        total = seats_total + extra
        invoice_id = f"in_{period_month:%Y%m}"
        payment_intent = f"pi_{period_month:%Y%m}"
        raised_ts = int(_dt.datetime.combine(period_month, _dt.time(), tzinfo=_dt.UTC).timestamp())
        read_at = _dt.datetime.combine(read_day, _dt.time(), tzinfo=_dt.UTC)
        period_end_ts = int(
            _dt.datetime.combine(
                (period_month + _dt.timedelta(days=32)).replace(day=1), _dt.time(), tzinfo=_dt.UTC
            ).timestamp()
        )

        envelope = {
            "_airbyte_extracted_at": read_at,
            "_airbyte_meta": "{}",
            "_airbyte_generation_id": 0,
            "tenant_id": tenant_uuid,
            "source_id": source_id,
            "collected_at": read_at.isoformat(),
            "data_source": "insight_claude_team",
            "chain_status": "ok",
            "invoice_id": invoice_id,
            "invoice_status": "paid",
            "invoice_created_ts": raised_ts,
            "invoice_currency": "usd",
        }

        rows.append(
            bronze_row(
                **envelope,
                _airbyte_raw_id=deterministic_uuid("ai.invoice.raw", invoice_id, "invoice"),
                # The connector's wrapper key, whose last part is a due date the
                # vendor does not report and which it stringifies as absent.
                unique_key=f"{tenant_uuid}-{source_id}-invoice-{raised_ts}-{payment_intent}-{total}-None",
                invoice_total=total,
                invoice_total_excluding_tax=total,
                invoice_num_seats=tiered_seats,
                invoice_payment_intent=payment_intent,
                # The span its lines charge for, as the connector reports it: the
                # invoice is raised at the boundary of the month it bills.
                period_start_ts=raised_ts,
                period_end_ts=period_end_ts,
            )
        )

        lines = [
            (
                "subscriptions",
                f"il_seats_{period_month:%Y%m}",
                f"{tiered_seats} x Example plan - Standard",
                seats_total,
                tiered_seats,
                _SEAT_PRICE_CENTS,
                _SEAT_PRICE_CENTS,
            ),
            (
                "overusage",
                f"il_extra_{period_month:%Y%m}",
                "Prepaid extra usage, Example plan",
                extra,
                1,
                extra,
                None,
            ),
        ]
        for category, line_id, description, amount, quantity, unit, seat_unit in lines:
            rows.append(
                bronze_row(
                    **envelope,
                    _airbyte_raw_id=deterministic_uuid("ai.invoice.raw", invoice_id, line_id),
                    unique_key=f"{tenant_uuid}-{source_id}-{invoice_id}-{line_id}",
                    line_id=line_id,
                    description=description,
                    product_name="Example plan - Standard",
                    tier_label="Standard" if category == "subscriptions" else None,
                    category=category,
                    is_proration=False,
                    amount=amount,
                    currency="usd",
                    quantity=quantity,
                    unit_amount=unit,
                    seat_unit_amount=seat_unit,
                    period_start_ts=raised_ts,
                    period_end_ts=period_end_ts,
                )
            )
    return bulk_insert(
        client, "bronze_claude_team_invoices", "claude_team_invoice_lines", cols, rows
    )


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    return {
        "silver.class_ai_overage": seed_ai_seat_overage(client, roster, tenant_uuid, days),
        "bronze_claude_team_invoices.claude_team_invoice_lines": seed_claude_team_invoices_bronze(
            client, roster, tenant_uuid, days
        ),
    }
