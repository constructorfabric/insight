"""An attendee who arrives in a later sync re-opens every day of that meeting.

A session's Zoom time counts only while someone else was in the meeting, so an
incremental build re-aggregates every date of each meeting touched by a recent
extract. alice is alone on Dec 20 until bob's row, joined on Dec 21, arrives in a
later sync; the incremental build then gives her Dec 20 the 20 minutes they shared.
"""

from __future__ import annotations

import pytest
from insight_datapath.ch_seeder import CHSeeder
from insight_datapath.clickhouse import client
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.instance import InstanceConfig
from insight_datapath.metric_expect import approx
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_meeting_hours_late_attendee"

ALICE = "alice@example.com"
REBUILT_MODELS = "zoom__collab_meeting_activity+"

BOB_LATE_FIELDS = {
    "_airbyte_raw_id": "00000000-0000-0000-0000-000000000009",
    "_airbyte_extracted_at": "2030-01-05T00:00:00",
    "unique_key": "late-bob-z9",
    "participant_uuid": "lb1",
    "email": "bob@example.com",
    "user_name": "Bob",
    "join_time": "2026-12-21T00:10:00Z",
    "leave_time": "2026-12-21T00:30:00Z",
}


def _alice_dec_20(spec: SpecRun) -> object:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-12-20", "to": "2026-12-20"},
                "metrics": [{"metric_key": "collab.meeting_hours", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200, "should answer 200 for alice on Dec 20"
    return r


def test_late_attendee_reopens_the_earlier_day(
    spec: SpecRun, instance_cfg: InstanceConfig, ch_seeder: CHSeeder, dbt_runner: DbtRunner
) -> None:
    before = _alice_dec_20(spec)
    before.row("collab.meeting_hours", "period", entity_id=ALICE).check(
        "value",
        lambda v: v is not None and float(v) == approx(0.0),
        "alice is alone before bob's row",
    )

    with client(instance_cfg) as connection:
        rows = connection.query(
            "SELECT * FROM bronze_zoom.participants FINAL WHERE unique_key = 'late-alice-z9'"
        )
        alice_row = next(rows.named_results())
    ch_seeder.seed_records("bronze_zoom", "participants", [{**alice_row, **BOB_LATE_FIELDS}])
    dbt_runner.run(REBUILT_MODELS)

    after = _alice_dec_20(spec)
    after.row("collab.meeting_hours", "period", entity_id=ALICE).check(
        "value",
        lambda v: v is not None and float(v) == approx(20 / 60),
        "alice's Dec 20 gains the 20 minutes shared with bob",
    )
