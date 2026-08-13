"""A billing month becomes durable at the sync that reads it, and stays.

The vendor reports spend as period-to-date with no month of its own, and the
connector's `unique_key` carries no month either. Bronze is promoted to
`ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key`, so it holds one
row per seat — whatever was read last. The month enters the key in staging, which
is where monthly history accumulates.

The YAML rig seeds bronze once and cannot express that: two months seeded at once
would model a state production never reaches, and would prove nothing about
accumulation. This drives the pipeline twice instead, replacing bronze between
runs the way a sync does.
"""

from __future__ import annotations

from lib import clickhouse
from lib.ch_seeder import CHSeeder
from lib.dbt_runner import DbtRunner
from lib.worker import WorkerContext

BRONZE_SCHEMA = "bronze_claude_team"
BRONZE_TABLE = "claude_team_overage_spend"
STAGING_SELECTOR = "+claude_team__ai_overage"
SILVER_SELECTOR = "class_ai_overage"

TENANT = "11111111-1111-1111-1111-111111111111"
SOURCE = "claude-team-history-test"
SEAT_EMAIL = "grace@example.com"
SEAT_UUID = "seat-grace"

# The seeder coerces each value to its ClickHouse column type, so a row is a flat
# map of scalars.
SeatSnapshot = dict[str, str | int | bool | None]


def _snapshot(read_at: str, used_credits: int) -> SeatSnapshot:
    """One seat's spend state as the endpoint returns it: current period to date."""
    return {
        "_airbyte_raw_id": "00000000-0000-0000-0000-000000000000",
        "_airbyte_extracted_at": read_at,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": TENANT,
        "source_id": SOURCE,
        # No month in the key — the seat's identity is all the connector emits.
        "unique_key": f"{TENANT}-{SOURCE}-{SEAT_UUID}",
        "collected_at": read_at,
        "data_source": "insight_claude_team",
        "account_uuid": SEAT_UUID,
        "account_email": SEAT_EMAIL,
        "seat_tier": "team_tier_1",
        "is_enabled": True,
        "monthly_credit_limit": 1000,
        "used_credits": used_credits,
        "currency": "USD",
        "used_credits_basis": "post_discount",
        "limit_type": "seat_tier",
    }


def _sync(ch_seeder: CHSeeder, dbt_runner: DbtRunner, worker_ctx: WorkerContext, row: SeatSnapshot) -> None:
    """One sync: bronze is replaced by the new snapshot, then the models run."""
    ch_seeder.seed_records(BRONZE_SCHEMA, BRONZE_TABLE, [row])
    dbt_runner.build(STAGING_SELECTOR, worker_ctx=worker_ctx)
    dbt_runner.build(SILVER_SELECTOR, worker_ctx=worker_ctx)


def test_a_month_survives_the_next_month_snapshot(
    ch_seeder: CHSeeder, dbt_runner: DbtRunner, worker_ctx: WorkerContext
) -> None:
    ch_seeder.truncate_touched()
    # Truncate up front, not only via the ledger: silver is incremental on
    # `_version > max(_version)`, so a row left by an earlier session with a
    # later read time would filter November's insert out and the first sync
    # would land nothing.
    for schema, table in (
        (BRONZE_SCHEMA, BRONZE_TABLE),
        ("staging", "claude_team__ai_overage"),
        ("silver", "class_ai_overage"),
    ):
        ch_seeder.truncate_table(schema, table)
        ch_seeder.ledger.record(schema, table)

    _sync(ch_seeder, dbt_runner, worker_ctx, _snapshot("2026-11-15T00:00:00Z", 500))
    _sync(ch_seeder, dbt_runner, worker_ctx, _snapshot("2026-12-10T00:00:00Z", 250))

    months = clickhouse.query(
        ch_seeder.cfg,
        f"""
        SELECT toString(period_month), used_amount_cents
        FROM silver.class_ai_overage FINAL
        WHERE email = '{SEAT_EMAIL}'
        ORDER BY period_month
        """,
    )

    assert months == [("2026-11-01", 500), ("2026-12-01", 250)], (
        f"December's snapshot must not take November's month with it: {months}"
    )
