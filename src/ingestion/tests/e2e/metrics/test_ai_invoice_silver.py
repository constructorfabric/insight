"""Bronze invoice rows reach `silver.class_ai_invoice` with their prices intact.

The YAML rig asserts through `/v1/metric-results`, which cannot cover this yet:
invoices have no metric key until `ai.seat_cost` lands. So this test asserts one
layer earlier — seed bronze, build the connector's models, read the class — which
is where every rule the connector encodes becomes visible or is lost:

  * a monthly invoice prices two tiers, each keeping its own per-seat amount
  * a proration keeps its money and carries no seat price
  * extra usage is `overusage` and prices no seat
  * an invoice whose chain failed keeps its total and contributes no line
  * an invoice's money is on its own row alone, so summing it needs no dedup
  * a row is dated by the period it charges for, not by the invoice date
  * an invoice enriched by a later sync replaces its own earlier row
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
# Its own source: the class keeps every test's rows for the session, and each
# recovery pair is only legible on its own.
SOURCE_ONE_BUILD = "claude-team-invoices-recovered-one-build"
SOURCE_TWO_BUILDS = "claude-team-invoices-recovered-two-builds"
SOURCE_SECOND_INSTANCE = "claude-team-invoices-second-instance"

# 2026-08-01 and 2026-09-01 UTC — the window a monthly line charges for.
AUG_START, SEP_START = 1785542400, 1788220800
# The invoice itself is raised at the boundary, a day before the period opens:
# dating by this instead of by the period would file the row in July.
RAISED_AT = 1785456000

# Staging admits only rows read strictly after what it already holds, and THAT
# watermark is table-wide. So every instant below is distinct and ascends in the
# order the fixtures run: reuse one and the watermark, not the model, decides which
# rows arrive — an assertion about the model would then hold with the model deleted.
READ_AT = "2026-09-02T00:00:00Z"
ONE_BUILD_BROKEN_AT = "2026-09-03T00:00:00Z"
ONE_BUILD_RECOVERED_AT = "2026-09-04T00:00:00Z"
TWO_BUILDS_BROKEN_AT = "2026-09-05T00:00:00Z"
TWO_BUILDS_RECOVERED_AT = "2026-09-06T00:00:00Z"

# The class's own boundary is per source instance, so this one deliberately does
# NOT ascend: it is read before every instant above, which is what a second
# connector instance stamping `_version` from its own clock looks like.
SECOND_INSTANCE_READ_AT = "2026-08-20T00:00:00Z"


def _base(source_id: str, read_at: str) -> dict:
    """Every column of the bronze table, all absent but the envelope.

    Bronze is rectangular: an invoice's own row leaves the line fields unset and a
    line row leaves the invoice's money unset, rather than either carrying zeroes.
    """
    return {
        "_airbyte_raw_id": "00000000-0000-0000-0000-000000000000",
        "_airbyte_extracted_at": read_at,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": TENANT,
        "source_id": source_id,
        "unique_key": None,
        "collected_at": read_at,
        "data_source": "insight_claude_team",
        "chain_status": "ok",
        "invoice_ref": None,
        "invoice_status": "paid",
        "invoice_created_ts": RAISED_AT,
        "invoice_currency": "usd",
        "invoice_id": None,
        "invoice_due_date_ts": None,
        "invoice_total": None,
        "invoice_total_excluding_tax": None,
        "invoice_num_seats": None,
        "invoice_payment_intent": None,
        "line_id": None,
        "description": None,
        "product_name": None,
        "tier_label": None,
        "category": None,
        "is_proration": None,
        "amount": None,
        "currency": None,
        "quantity": None,
        "unit_amount": None,
        "seat_unit_amount": None,
        "period_start_ts": None,
        "period_end_ts": None,
    }


def _invoice_row(
    intent: str,
    total: int,
    net: int,
    *,
    ref: str | None = None,
    source_id: str = SOURCE,
    read_at: str = READ_AT,
    **over,
) -> dict:
    """An invoice's own row.

    Keyed on the identity decoded out of its hosted URL when the vendor offered
    one: that identity reads the same on every sync, which is what lets a later
    sync replace this row instead of adding a second one beside it. With no URL
    there is no identity, so the key falls back to what the wrapper reports —
    both branches are exercised by the fixture below.
    """
    row = _base(source_id, read_at)
    key = f"invoice-{ref}" if ref else f"invoice-{RAISED_AT}-{intent}-{total}-None"
    row.update(
        unique_key=f"{TENANT}-{source_id}-{key}",
        invoice_ref=ref,
        invoice_payment_intent=intent,
        invoice_total=total,
        invoice_total_excluding_tax=net,
    )
    row.update(over)
    return row


def _line_row(
    invoice_id: str, line_id: str, *, ref: str | None = None, source_id: str = SOURCE, read_at: str = READ_AT, **over
) -> dict:
    """One line's row: line money only, keyed on Stripe's own ids.

    It carries its invoice's identity too, so a line can be tied back to the
    invoice's row without going through an id the chain may not have reached.
    """
    row = _base(source_id, read_at)
    row.update(
        unique_key=f"{TENANT}-{source_id}-{invoice_id}-{line_id}",
        invoice_ref=ref,
        invoice_id=invoice_id,
        line_id=line_id,
        category="subscriptions",
        is_proration=False,
        currency="usd",
        period_start_ts=AUG_START,
        period_end_ts=SEP_START,
    )
    row.update(over)
    return row


BRONZE_ROWS = [
    # One monthly invoice pricing two tiers at once — the shape that makes an
    # organisation-wide seat price wrong. Its own row carries the money; the two
    # lines carry only theirs, and 1_000 + 15_000 is the invoice's net total.
    _invoice_row(
        "pi_monthly",
        16_500,
        16_000,
        ref="acct_EXAMPLE,_monthly",
        invoice_id="in_MONTHLY",
        invoice_num_seats=1,
        period_start_ts=AUG_START,
        period_end_ts=SEP_START,
    ),
    _line_row(
        "in_MONTHLY",
        "il_standard",
        ref="acct_EXAMPLE,_monthly",
        tier_label="Standard",
        product_name="Example plan - Standard",
        description="1 x Example plan - Standard",
        amount=1_000,
        quantity=1,
        unit_amount=1_000,
        seat_unit_amount=1_000,
    ),
    _line_row(
        "in_MONTHLY",
        "il_premium",
        ref="acct_EXAMPLE,_monthly",
        tier_label="Premium",
        product_name="Example plan - Premium",
        description="5 x Example plan - Premium",
        amount=15_000,
        quantity=5,
        unit_amount=3_000,
        seat_unit_amount=3_000,
    ),
    # A mid-period seat change: real money, no unit price.
    _invoice_row(
        "pi_prorate",
        -1_500,
        -1_500,
        ref="acct_EXAMPLE,_prorate",
        invoice_id="in_PRORATE",
        period_start_ts=AUG_START,
        period_end_ts=SEP_START,
    ),
    _line_row(
        "in_PRORATE",
        "il_unused",
        ref="acct_EXAMPLE,_prorate",
        description="Unused time on 5 x Example plan - Premium",
        amount=-1_500,
        quantity=5,
        is_proration=True,
    ),
    # Prepaid extra usage — the invoiced counterpart of used_credits.
    _invoice_row(
        "pi_prepaid",
        2_000,
        2_000,
        ref="acct_EXAMPLE,_prepaid",
        invoice_id="in_PREPAID",
        period_start_ts=AUG_START,
        period_end_ts=SEP_START,
    ),
    _line_row(
        "in_PREPAID",
        "il_prepaid",
        ref="acct_EXAMPLE,_prepaid",
        category="overusage",
        description="Prepaid extra usage, Example plan",
        amount=2_000,
        quantity=1,
        unit_amount=2_000,
    ),
    # An invoice whose chain never completed: the ledger survives, the price does
    # not, and with no line there is only the raise date to file it by.
    _invoice_row("pi_broken", 999, 999, ref="acct_EXAMPLE,_broken", chain_status="failed", invoice_currency=None),
    # A finalised invoice the vendor offered no hosted URL for. Not a draft: the
    # connector skips those, so one could never reach bronze in the first place.
    _invoice_row("pi_no_url", 750, 750, chain_status="no_hosted_url", invoice_status="open"),
]

COLUMNS = [
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

_SELECT = (
    "SELECT unique_key, invoice_id, line_id, category, tier_label, is_proration, "
    "chain_status, toString(period_month), amount_cents, seat_unit_cents, "
    "seat_quantity, invoice_net_cents, currency "
    "FROM silver.class_ai_invoice FINAL "
)


def _seed_and_build(ch_seeder: CHSeeder, dbt_runner: DbtRunner, worker_ctx: WorkerContext, rows: list[dict]) -> None:
    schema_file = Path(__file__).parent / "schemas" / f"{TABLE}.yaml"
    schemas = yaml.safe_load(schema_file.read_text(encoding="utf-8"))["schemas"]
    ch_seeder.seed_bronze({TABLE: rows}, schemas)
    dbt_runner.build(SELECTOR, worker_ctx=worker_ctx)


def _read_class(cfg: SessionConfig, source_id: str, order_by: str = "unique_key") -> list[dict]:
    rows = clickhouse.query(cfg, f"{_SELECT}WHERE source_id = '{source_id}' ORDER BY {order_by}")
    return [dict(zip(COLUMNS, row)) for row in rows]


@pytest.fixture
def invoice_silver(
    ch_migrations_applied: SessionConfig, ch_seeder: CHSeeder, dbt_runner: DbtRunner, worker_ctx: WorkerContext
) -> list[dict]:
    """Seed bronze, build the connector's models, read the class back."""
    _seed_and_build(ch_seeder, dbt_runner, worker_ctx, BRONZE_ROWS)
    return _read_class(ch_migrations_applied, SOURCE)


def _by_line(rows: list[dict], line_id: str) -> dict:
    return next(r for r in rows if r["line_id"] == line_id)


def _by_status(rows: list[dict], chain_status: str) -> dict:
    return next(r for r in rows if r["chain_status"] == chain_status)


def test_every_bronze_row_reaches_the_class(invoice_silver):
    assert len(invoice_silver) == len(BRONZE_ROWS)


def test_each_tier_keeps_its_own_seat_price(invoice_silver):
    """One invoice, two tiers — an organisation-wide price would lose one of them."""
    standard = _by_line(invoice_silver, "il_standard")
    premium = _by_line(invoice_silver, "il_premium")
    assert (standard["seat_unit_cents"], standard["seat_quantity"]) == (1_000, 1)
    assert (premium["seat_unit_cents"], premium["seat_quantity"]) == (3_000, 5)
    assert standard["tier_label"] == "Standard" and premium["tier_label"] == "Premium"
    assert standard["invoice_id"] == premium["invoice_id"] == "in_MONTHLY"


def test_a_proration_keeps_its_money_and_prices_no_seat(invoice_silver):
    row = _by_line(invoice_silver, "il_unused")
    assert row["amount_cents"] == -1_500, "a credit stays on the ledger"
    assert row["category"] == "subscriptions"
    assert row["is_proration"] == 1
    assert row["seat_unit_cents"] is None


def test_extra_usage_is_overusage_and_prices_no_seat(invoice_silver):
    row = _by_line(invoice_silver, "il_prepaid")
    assert row["category"] == "overusage"
    assert row["seat_unit_cents"] is None
    assert row["amount_cents"] == 2_000


def test_an_invoices_money_sits_on_its_own_row_alone(invoice_silver):
    """Carried on each line instead, one invoice counts once per line it has."""
    lines = [r for r in invoice_silver if r["line_id"]]
    invoices = [r for r in invoice_silver if not r["line_id"]]
    assert all(r["invoice_net_cents"] is None for r in lines), "a line prices itself only"
    assert sum(r["invoice_net_cents"] for r in invoices) == 18_249, "16000 - 1500 + 2000 + 999 + 750"


def test_a_failed_chain_keeps_the_invoice_and_no_line(invoice_silver):
    row = _by_status(invoice_silver, "failed")
    assert row["invoice_net_cents"] == 999, "the money is still on the ledger"
    assert row["line_id"] is None and row["invoice_id"] is None
    assert row["seat_unit_cents"] is None
    assert row["currency"] == "usd", "falls back rather than emitting an empty currency"


def test_an_invoice_with_no_hosted_url_reaches_the_class_as_its_own_state(invoice_silver):
    """Reading a missing URL as a format change would fail the whole sync instead."""
    row = _by_status(invoice_silver, "no_hosted_url")
    assert row["invoice_net_cents"] == 750
    assert row["line_id"] is None and row["invoice_id"] is None


def test_a_row_is_dated_by_the_period_it_charges_for(invoice_silver):
    """The invoice is raised on 2026-07-31; it and its lines charge for August."""
    assert _by_line(invoice_silver, "il_premium")["period_month"] == "2026-08-01"
    monthly = next(r for r in invoice_silver if r["invoice_id"] == "in_MONTHLY" and not r["line_id"])
    assert monthly["period_month"] == "2026-08-01", "the invoice's money is filed with its lines"


def test_an_invoice_with_no_line_falls_back_to_the_day_it_was_raised(invoice_silver):
    assert _by_status(invoice_silver, "failed")["period_month"] == "2026-07-01"


# A recovery: the sync that failed and the sync that succeeded describe one
# invoice, so the class has to end up with one row for it and not two. The pair
# below is the same recovery twice — reached inside a single build, and reached
# across two, which is the case an append-only staging model gets wrong. Both
# syncs read the same identity out of the URL; that is what makes them one row.
RECOVERED_REF = "acct_EXAMPLE,_recovered"


def _broken_row(source_id: str, read_at: str) -> dict:
    """Carries the identity: a chain only fails once the URL has decoded."""
    return _invoice_row(
        "pi_recovered", 16_500, 16_000, ref=RECOVERED_REF, source_id=source_id, read_at=read_at, chain_status="failed"
    )


def _recovered_rows(source_id: str, read_at: str) -> list[dict]:
    return [
        _invoice_row(
            "pi_recovered",
            16_500,
            16_000,
            ref=RECOVERED_REF,
            source_id=source_id,
            read_at=read_at,
            invoice_id="in_RECOVERED",
            period_start_ts=AUG_START,
            period_end_ts=SEP_START,
        ),
        _line_row(
            "in_RECOVERED",
            "il_late",
            ref=RECOVERED_REF,
            source_id=source_id,
            read_at=read_at,
            tier_label="Standard",
            description="1 x Example plan - Standard",
            amount=1_000,
            quantity=1,
            unit_amount=1_000,
            seat_unit_amount=1_000,
        ),
    ]


@pytest.fixture
def recovered_in_one_build(
    ch_migrations_applied: SessionConfig, ch_seeder: CHSeeder, dbt_runner: DbtRunner, worker_ctx: WorkerContext
) -> list[dict]:
    """Both syncs' rows already in bronze when the model first runs."""
    rows = [
        _broken_row(SOURCE_ONE_BUILD, ONE_BUILD_BROKEN_AT),
        *_recovered_rows(SOURCE_ONE_BUILD, ONE_BUILD_RECOVERED_AT),
    ]
    _seed_and_build(ch_seeder, dbt_runner, worker_ctx, rows)
    return _read_class(ch_migrations_applied, SOURCE_ONE_BUILD, order_by="line_id NULLS FIRST")


@pytest.fixture
def recovered_across_two_builds(
    ch_migrations_applied: SessionConfig, ch_seeder: CHSeeder, dbt_runner: DbtRunner, worker_ctx: WorkerContext
) -> list[dict]:
    """The gap built into the class first, the enrichment only on the next run.

    This is the real sequence: a nightly sync fails, the model runs, and only the
    following night does the chain complete. The gap row is in the class before the
    enrichment is even read, so nothing the second build selects can filter it out.
    """
    _seed_and_build(ch_seeder, dbt_runner, worker_ctx, [_broken_row(SOURCE_TWO_BUILDS, TWO_BUILDS_BROKEN_AT)])
    _seed_and_build(ch_seeder, dbt_runner, worker_ctx, _recovered_rows(SOURCE_TWO_BUILDS, TWO_BUILDS_RECOVERED_AT))
    return _read_class(ch_migrations_applied, SOURCE_TWO_BUILDS, order_by="line_id NULLS FIRST")


def test_a_recovery_inside_one_build_leaves_no_gap_behind(recovered_in_one_build):
    assert [r["chain_status"] for r in recovered_in_one_build] == ["ok", "ok"]
    assert [r["line_id"] for r in recovered_in_one_build] == [None, "il_late"]
    assert [r["invoice_id"] for r in recovered_in_one_build] == ["in_RECOVERED", "in_RECOVERED"]


def test_a_recovery_across_two_builds_leaves_no_gap_behind(recovered_across_two_builds):
    """Otherwise the invoice's money is counted twice and coverage stays red."""
    assert [r["chain_status"] for r in recovered_across_two_builds] == ["ok", "ok"]
    assert [r["line_id"] for r in recovered_across_two_builds] == [None, "il_late"]
    invoice = recovered_across_two_builds[0]
    assert invoice["invoice_id"] == "in_RECOVERED", "the gap row became the enriched one"
    assert invoice["invoice_net_cents"] == 16_000
    assert recovered_across_two_builds[1]["invoice_net_cents"] is None, "counted once, not twice"


@pytest.fixture
def second_instance_silver(
    ch_migrations_applied: SessionConfig, ch_seeder: CHSeeder, dbt_runner: DbtRunner, worker_ctx: WorkerContext
) -> list[dict]:
    """A second source instance, read BEFORE everything the class already holds."""
    rows = [
        _invoice_row(
            "pi_second",
            4_200,
            4_000,
            ref="acct_EXAMPLE,_second",
            source_id=SOURCE_SECOND_INSTANCE,
            read_at=SECOND_INSTANCE_READ_AT,
            invoice_id="in_SECOND",
            period_start_ts=AUG_START,
            period_end_ts=SEP_START,
        )
    ]
    _seed_and_build(ch_seeder, dbt_runner, worker_ctx, rows)
    return _read_class(ch_migrations_applied, SOURCE_SECOND_INSTANCE)


def test_a_second_source_instance_is_not_shut_out_by_the_first(second_instance_silver):
    """The class is written by every connector feeding it, each stamping `_version`
    from its own clock. A boundary taken over the whole table would leave this
    instance's rows below whatever the first instance committed — forever, and
    silently. The boundary is per instance, so an instance the class has never
    seen has no boundary to clear."""
    assert [r["invoice_id"] for r in second_instance_silver] == ["in_SECOND"]
    assert second_instance_silver[0]["invoice_net_cents"] == 4_000
