from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest
from insight_datapath import clickhouse
from insight_datapath.ch_seeder import CHSeeder
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.fixture_loader import load
from insight_datapath.instance import InstanceConfig
from insight_datapath.reset import clear
from insight_datapath.tracked_models import TrackedModels

pytestmark = pytest.mark.fixture

SPEC = "collab_meetings_zoom_full_refresh"


def test_full_refresh_keeps_old_sessions_and_stitches_their_reconnects(
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    instance_cfg: InstanceConfig,
    tenant: str,
) -> None:
    fixture = load(Path(__file__).with_name(f"{SPEC}.test.yaml"), substitutions={"tenant": tenant})
    extracted_at = datetime.now(tz=UTC)
    for rows in fixture.bronze.values():
        for row in rows:
            age_days = 120 if row["unique_key"].startswith("old-") else 1
            row["_airbyte_extracted_at"] = (extracted_at - timedelta(days=age_days)).isoformat()

    clear(ch_seeder.cfg, ch_seeder.ledger.drain())
    ch_seeder.seed_bronze(fixture.bronze, fixture.schemas)
    tracked = TrackedModels(dbt_runner, ch_seeder)
    tracked.run(["zoom__meeting_sessions", "zoom__collab_meeting_activity"], full_refresh=True)

    sessions = clickhouse.query(
        instance_cfg,
        "SELECT uuid, logical_meeting_id FROM staging.zoom__meeting_sessions FINAL ORDER BY uuid",
    )
    assert [uuid for uuid, _ in sessions] == ["old-a", "old-b", "recent"], (
        "a full refresh must rebuild old sessions alongside the recent control"
    )
    logical_ids = dict(sessions)
    assert logical_ids["old-a"] == logical_ids["old-b"], "old reconnects share one logical meeting"
    assert logical_ids["old-a"] != logical_ids["recent"], "the control is a separate meeting"

    activity = clickhouse.query(
        instance_cfg,
        "SELECT toString(date), person_key, meetings_attended, attended_meeting_keys"
        " FROM staging.zoom__collab_meeting_activity FINAL ORDER BY date, person_key",
    )
    assert activity == [
        ("2026-01-10", "alice@example.com", 1, [logical_ids["old-a"]]),
        ("2026-01-10", "bob@example.com", 1, [logical_ids["old-a"]]),
        ("2026-06-10", "alice@example.com", 1, [logical_ids["recent"]]),
        ("2026-06-10", "bob@example.com", 1, [logical_ids["recent"]]),
    ], "old reconnects must count as one meeting per person, with the recent meeting intact"
