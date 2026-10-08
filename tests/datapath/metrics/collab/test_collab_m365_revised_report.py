"""A re-synced Microsoft 365 report day serves its latest version.

Bronze keeps every version of a report day under one key until a background merge,
so staging reads it with FINAL and only the latest version reaches silver and gold,
whatever the merge state. The test stops merges, re-syncs a day with a new count and
writes the earlier version again, and the served value is still the latest one.
"""

from __future__ import annotations

import pytest
from insight_datapath.ch_seeder import CHSeeder
from insight_datapath.clickhouse import client, execute
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.instance import InstanceConfig
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_m365_revised_report"

ALICE = "alice@example.com"
BRONZE = "bronze_m365.email_activity"
REBUILT_MODELS = (
    "m365__collab_email_activity class_collab_email_activity "
    "collab_metric_evidence collab_metric_observations"
)


def test_latest_version_of_a_day_wins_before_any_merge(
    spec: SpecRun,
    instance_cfg: InstanceConfig,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
) -> None:
    """Dec 2 is re-synced from 0 to 9 and the old version is written again: 9 is served."""
    execute(instance_cfg, f"SYSTEM STOP MERGES {BRONZE}")
    try:
        with client(instance_cfg) as connection:
            rows = connection.query(
                f"SELECT * FROM {BRONZE} FINAL WHERE unique_key = 'revised-alice-20261202'"
            )
            earlier = next(rows.named_results())

        latest = {**earlier, "sendCount": 9, "_airbyte_extracted_at": "2026-12-06T00:00:00"}
        ch_seeder.seed_records("bronze_m365", "email_activity", [latest])
        ch_seeder.seed_records("bronze_m365", "email_activity", [earlier])
        dbt_runner.run(REBUILT_MODELS)

        r = spec.call(
            {
                "url": "/v1/metric-results",
                "method": "POST",
                "body": {
                    "entity": {"type": "person", "ids": [ALICE]},
                    "period": {"from": "2026-12-02", "to": "2026-12-02"},
                    "metrics": [
                        {"metric_key": "collab.emails_sent", "views": [{"view": "period"}]}
                    ],
                },
            }
        )
        assert r.status == 200
        r.row("collab.emails_sent", "period", entity_id=ALICE).equals(value=9)
    finally:
        execute(instance_cfg, f"SYSTEM START MERGES {BRONZE}")
