"""Bronze invoice lines reach `silver.class_ai_invoice` with their prices intact.

The YAML rig asserts through `/v1/metric-results`, which cannot cover this yet:
invoices have no metric key until `ai.seat_cost` lands. So this test asserts one
layer earlier — seed bronze, build the connector's models, read the class — which
is where every rule the connector encodes becomes visible or is lost:

  * a monthly invoice prices two tiers, each keeping its own per-seat amount
  * a proration keeps its money and carries no seat price
  * extra usage is `overusage` and prices no seat
  * an invoice whose chain failed keeps its total and contributes no line
  * a line is dated by the period it charges for, not by the invoice date
"""

from __future__ import annotations

from pathlib import Path

import pytest
import yaml
from lib import clickhouse
from lib.ch_seeder import CHSeeder
from lib.config import SessionConfig
from lib.dbt_runner import DbtRunner
from lib.worker import WorkerContext

TABLE = "bronze_claude_team_invoices.claude_team_invoice_lines"
SELECTOR = "tag:claude-team-invoices+"

TENANT = "11111111-1111-1111-1111-111111111111"
SOURCE = "claude-team-invoices-test"

# 2026-08-01 and 2026-09-01 UTC — the window a monthly line charges for.
AUG_START, SEP_START = 1754006400, 1756684800
# The invoice itself is raised at the boundary, a day before the period opens:
# dating by this instead of by the period would file the line in July.
RAISED_AT = 1753920000


def _row(**over):
    base = {
        "_airbyte_raw_id": "00000000-0000-0000-0000-000000000000",
        "_airbyte_extracted_at": "2026-09-02T00:00:00Z",
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": TENANT,
        "source_id": SOURCE,
        "unique_key": None,
        "collected_at": "2026-09-02T00:00:00Z",
        "data_source": "insight_claude_team",
        "chain_status": "ok",
        "invoice_id": "in_MONTHLY",
        "invoice_status": "paid",
        "invoice_created_ts": RAISED_AT,
        "invoice_due_date_ts": None,
        "invoice_currency": "usd",
        "invoice_total": 1595000,
        "invoice_total_excluding_tax": 1550000,
        "invoice_num_seats": 18,
        "invoice_payment_intent": "pi_monthly",
        "line_id": None,
        "description": None,
        "product_name": None,
        "tier_label": None,
        "category": "subscriptions",
        "is_proration": False,
        "amount": None,
        "currency": "usd",
        "quantity": None,
        "unit_amount": None,
        "seat_unit_amount": None,
        "period_start_ts": AUG_START,
        "period_end_ts": SEP_START,
    }
    base.update(over)
    return base


BRONZE_ROWS = [
    # One monthly invoice pricing two tiers at once — the shape that makes an
    # organisation-wide seat price wrong.
    _row(
        unique_key=f"{TENANT}-{SOURCE}-in_MONTHLY-il_standard",
        line_id="il_standard",
        tier_label="Standard",
        description="18 x Team plan - Standard (at $25.00 / month)",
        amount=45000,
        quantity=18,
        unit_amount=2500,
        seat_unit_amount=2500,
    ),
    _row(
        unique_key=f"{TENANT}-{SOURCE}-in_MONTHLY-il_premium",
        line_id="il_premium",
        tier_label="Premium",
        description="124 x Team plan - Premium (at $125.00 / month)",
        amount=1550000,
        quantity=124,
        unit_amount=12500,
        seat_unit_amount=12500,
    ),
    # A mid-period seat change: real money, no unit price.
    _row(
        unique_key=f"{TENANT}-{SOURCE}-in_PRORATE-il_unused",
        invoice_id="in_PRORATE",
        invoice_payment_intent="pi_prorate",
        invoice_total_excluding_tax=18758,
        line_id="il_unused",
        description="Unused time on 124 x Team plan - Premium",
        amount=-1163029,
        quantity=124,
        unit_amount=None,
        seat_unit_amount=None,
        is_proration=True,
    ),
    # Prepaid extra usage — the invoiced counterpart of used_credits.
    _row(
        unique_key=f"{TENANT}-{SOURCE}-in_PREPAID-il_prepaid",
        invoice_id="in_PREPAID",
        invoice_payment_intent="pi_prepaid",
        invoice_total_excluding_tax=210000,
        line_id="il_prepaid",
        category="overusage",
        description="Prepaid extra usage, Team plan",
        amount=210000,
        quantity=1,
        unit_amount=210000,
        seat_unit_amount=None,
    ),
    # An invoice whose chain never completed: the ledger survives, the price does not.
    _row(
        unique_key=f"{TENANT}-{SOURCE}-failed-{RAISED_AT}-pi_broken",
        chain_status="failed",
        invoice_id=None,
        invoice_payment_intent="pi_broken",
        invoice_total_excluding_tax=99900,
        category=None,
        is_proration=None,
        currency=None,
        period_start_ts=None,
    ),
]


@pytest.fixture
def invoice_silver(
    ch_migrations_applied: SessionConfig,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    worker_ctx: WorkerContext,
) -> list[dict]:
    """Seed bronze, build the connector's models, read the class back."""
    schema_file = Path(__file__).parent / "schemas" / f"{TABLE}.yaml"
    schemas = yaml.safe_load(schema_file.read_text(encoding="utf-8"))["schemas"]

    ch_seeder.seed_bronze({TABLE: BRONZE_ROWS}, schemas)
    dbt_runner.build(SELECTOR, worker_ctx=worker_ctx)

    # `query` returns tuples; name them here so the assertions read as facts
    # about columns rather than about positions.
    columns = [
        "unique_key",
        "invoice_id",
        "line_id",
        "category",
        "tier_label",
        "is_proration",
        "chain_status",
        "period_month",
        "amount_cents",
        "seat_unit_cents",
        "seat_quantity",
        "invoice_net_cents",
        "currency",
    ]
    rows = clickhouse.query(
        ch_migrations_applied,
        "SELECT unique_key, invoice_id, line_id, category, tier_label, is_proration, "
        "chain_status, toString(period_month), amount_cents, seat_unit_cents, "
        "seat_quantity, invoice_net_cents, currency "
        "FROM silver.class_ai_invoice FINAL "
        f"WHERE source_id = '{SOURCE}' ORDER BY unique_key",
    )
    return [dict(zip(columns, row)) for row in rows]


def _by_line(rows: list[dict], line_id: str) -> dict:
    return next(r for r in rows if r["line_id"] == line_id)


def test_every_bronze_line_reaches_the_class(invoice_silver):
    assert len(invoice_silver) == len(BRONZE_ROWS)


def test_each_tier_keeps_its_own_seat_price(invoice_silver):
    """One invoice, two tiers — an organisation-wide price would lose one of them."""
    standard = _by_line(invoice_silver, "il_standard")
    premium = _by_line(invoice_silver, "il_premium")
    assert (standard["seat_unit_cents"], standard["seat_quantity"]) == (2500, 18)
    assert (premium["seat_unit_cents"], premium["seat_quantity"]) == (12500, 124)
    assert standard["tier_label"] == "Standard" and premium["tier_label"] == "Premium"
    assert standard["invoice_id"] == premium["invoice_id"] == "in_MONTHLY"


def test_a_proration_keeps_its_money_and_prices_no_seat(invoice_silver):
    row = _by_line(invoice_silver, "il_unused")
    assert row["amount_cents"] == -1163029, "a credit stays on the ledger"
    assert row["category"] == "subscriptions"
    assert row["is_proration"] == 1
    assert row["seat_unit_cents"] is None


def test_extra_usage_is_overusage_and_prices_no_seat(invoice_silver):
    row = _by_line(invoice_silver, "il_prepaid")
    assert row["category"] == "overusage"
    assert row["seat_unit_cents"] is None
    assert row["amount_cents"] == 210000


def test_a_failed_chain_keeps_the_invoice_and_no_line(invoice_silver):
    row = next(r for r in invoice_silver if r["chain_status"] == "failed")
    assert row["invoice_net_cents"] == 99900, "the money is still on the ledger"
    assert row["line_id"] is None and row["invoice_id"] is None
    assert row["seat_unit_cents"] is None
    assert row["currency"] == "usd", "falls back rather than emitting an empty currency"


def test_a_line_is_dated_by_the_period_it_charges_for(invoice_silver):
    """The invoice is raised on 2026-07-31; its lines charge for August."""
    assert _by_line(invoice_silver, "il_premium")["period_month"] == "2026-08-01"
