"""Unified tasks.closed coverage for re-synced issue snapshots.

Bronze: four Jira issues, each extracted twice. One moves In Progress to Closed across
two extractions, one is a byte-identical re-sync, one has its later Closed snapshot
inserted before the older In Progress one, and one was Closed but the latest
extraction reopens it. The latest extraction wins regardless of insertion order,
identical snapshots count once, and a reopened issue is not counted as closed.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_closed_resync_dedup"

ALICE = "alice@example.com"


def test_tasks_closed_resync_deduplication(spec: SpecRun) -> None:
    """Three closes over the window, one per day for the first three issues; the
    reopened issue's day serves null."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-12-20", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "tasks.closed",
                        "views": [{"view": "period"}, {"view": "timeseries", "bucket": "day"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.closed", "period", entity_id=ALICE).equals(value=3)
    series = r.row("tasks.closed", "timeseries", entity_id=ALICE)
    series.contains(points={"bucket_start": "2026-12-21", "value": 1})
    series.contains(points={"bucket_start": "2026-12-22", "value": 1})
    series.contains(points={"bucket_start": "2026-12-23", "value": 1})
    series.contains(points={"bucket_start": "2026-12-24", "value": None})
