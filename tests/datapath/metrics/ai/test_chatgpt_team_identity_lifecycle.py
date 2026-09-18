"""A ChatGPT Team seat's identity follows the seat's lifecycle, in both directions.

`deactivated_time` is the only departure signal this source offers. A deactivated
account must stop asserting the email and display name it used to claim, or a
person keeps resolving through a seat the vendor has closed.

The reverse matters as much and is the half a deactivation condition alone cannot
express. `identity_inputs_from_history` re-emits an identity UPSERT only when the
field's own value changes, so an account deleted on deactivation and reactivated
with the SAME address would never have its email restored. The projection
therefore blanks the identity values while the seat is deactivated, which makes
the return read as a change from '' back to the address — so the observation
comes back without the address itself having to change.

The YAML rig seeds bronze once and cannot express a lifecycle. This drives the
pipeline three times instead, replacing bronze between runs the way a sync does.
"""

from __future__ import annotations

from insight_datapath import clickhouse
from insight_datapath.ch_seeder import CHSeeder
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.reset import clear

BRONZE_SCHEMA = "bronze_chatgpt_team"
BRONZE_TABLE = "chatgpt_team_seats"
IDENTITY_SELECTOR = "+chatgpt_team__identity_inputs"

SOURCE = "chatgpt-team-lifecycle-test"
SEAT_ID = "seat-heidi"
SEAT_EMAIL = "heidi@example.com"
SEAT_NAME = "Heidi Example"

Seat = dict[str, str | int | bool | None]


def _seat(tenant: str, read_at: str, deactivated_time: str | None) -> Seat:
    """The roster row as the endpoint returns it, at one read."""
    return {
        "_airbyte_raw_id": "00000000-0000-0000-0000-000000000000",
        "_airbyte_extracted_at": read_at,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": tenant,
        "source_id": SOURCE,
        "unique_key": f"{tenant}-{SOURCE}-{SEAT_ID}",
        "collected_at": read_at,
        "data_source": "insight_chatgpt_team",
        "user_id": SEAT_ID,
        "email": SEAT_EMAIL,
        "name": SEAT_NAME,
        "role": "standard-user",
        "seat_type": "default",
        "added_at": "2026-01-05T09:00:00Z",
        "account_user_id": f"account-{SEAT_ID}",
        "verified_email": None,
        "trial_state": None,
        "is_trial": False,
        "is_scim_managed": False,
        "creation_source": None,
        # The signal under test. The address and the display name never change.
        "deactivated_time": deactivated_time,
        "pending_seat_type": None,
        "reclaimable_seat_type": None,
        "credit_limits": None,
    }


def _sync(ch_seeder: CHSeeder, dbt_runner: DbtRunner, row: Seat) -> None:
    """One sync: bronze is replaced by the new roster state, then the models run.

    Replaced, not added to: the promoted relation dedups by unique_key only once
    its parts merge, so a second roster state left beside the first would let the
    snapshot hash whichever row it happened to read.
    """
    clear(ch_seeder.cfg, [(BRONZE_SCHEMA, BRONZE_TABLE)])
    ch_seeder.seed_records(BRONZE_SCHEMA, BRONZE_TABLE, [row])
    dbt_runner.build(IDENTITY_SELECTOR)


def _observations(ch_seeder: CHSeeder, value_type: str) -> list[tuple[str, str]]:
    """Every observation of one identity field for the seat, oldest first."""
    rows = clickhouse.query(
        ch_seeder.cfg,
        f"""
        SELECT operation_type, value
        FROM staging.chatgpt_team__identity_inputs
        WHERE source_account_id = {clickhouse.literal(SEAT_ID)}
          AND value_type = {clickhouse.literal(value_type)}
        ORDER BY _version, operation_type
        """,
    )
    return [(str(op), str(value)) for op, value in rows]


def test_a_seat_stops_and_resumes_asserting_its_identity(
    ch_seeder: CHSeeder, dbt_runner: DbtRunner, tenant: str
) -> None:
    clear(ch_seeder.cfg, ch_seeder.ledger.drain())
    ch_seeder.clear_and_record(
        (
            (BRONZE_SCHEMA, BRONZE_TABLE),
            ("staging", "chatgpt_team__seats_latest"),
            ("staging", "chatgpt_team__seats_snapshot"),
            ("staging", "chatgpt_team__seats_fields_history"),
            ("staging", "chatgpt_team__identity_inputs"),
        )
    )

    # Active: the seat claims an address and a name.
    _sync(ch_seeder, dbt_runner, _seat(tenant, "2026-11-01T00:00:00Z", None))

    email_after_active = _observations(ch_seeder, "email")
    assert email_after_active == [("UPSERT", SEAT_EMAIL)], (
        f"an active seat must assert its address: {email_after_active}"
    )

    # Deactivated: the vendor stamps deactivated_time. Nothing else changes.
    _sync(ch_seeder, dbt_runner, _seat(tenant, "2026-11-02T00:00:00Z", "2026-11-02T08:00:00Z"))

    email_after_deactivation = _observations(ch_seeder, "email")
    assert email_after_deactivation[-1] == ("DELETE", ""), (
        "a deactivated seat must stop asserting its address, or a person keeps "
        f"resolving through a closed seat: {email_after_deactivation}"
    )
    name_after_deactivation = _observations(ch_seeder, "display_name")
    assert name_after_deactivation[-1] == ("DELETE", ""), (
        f"the display name must close with the seat: {name_after_deactivation}"
    )
    id_after_deactivation = _observations(ch_seeder, "id")
    assert ("DELETE", "") in id_after_deactivation, (
        "the ADR-0002 binding row must be revoked too — no field change can "
        f"express that, which is what the deactivation condition is for: {id_after_deactivation}"
    )

    # Reactivated: deactivated_time is cleared. The address is the SAME one, so
    # nothing but the lifecycle signal has changed.
    _sync(ch_seeder, dbt_runner, _seat(tenant, "2026-11-03T00:00:00Z", None))

    email_after_reactivation = _observations(ch_seeder, "email")
    assert email_after_reactivation[-1] == ("UPSERT", SEAT_EMAIL), (
        "a reactivated seat must assert its address again even though the address "
        "never changed — a one-way delete would strand the account forever: "
        f"{email_after_reactivation}"
    )
    name_after_reactivation = _observations(ch_seeder, "display_name")
    assert name_after_reactivation[-1] == ("UPSERT", SEAT_NAME), (
        f"the display name must come back with the seat: {name_after_reactivation}"
    )
