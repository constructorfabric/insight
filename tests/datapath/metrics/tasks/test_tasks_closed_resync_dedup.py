"""Unified tasks.closed coverage for re-synced issue snapshots.

Bronze: five Jira issues, each extracted twice. One moves In Progress to Closed across
two extractions, one is a byte-identical re-sync, one has its later Closed snapshot
inserted before the older In Progress one, one was Closed but the latest extraction
reopens it, and bob's Bug arrives twice whole — snapshot and close event alike. The
latest extraction wins regardless of insertion order, identical snapshots count once,
a reopened issue is not counted as closed, and a re-synced bug fixes exactly once.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "tasks_closed_resync_dedup"

ALICE = "alice@example.com"
BOB = "bob@example.com"


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


def test_resynced_bug_scores_bugs_fixed_once(spec: SpecRun) -> None:
    """Bob's DDP-5 Bug arrives twice — issue snapshot and close event alike, under
    fresh extraction stamps — and bugs_fixed still counts a single fix."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [BOB]},
                "period": {"from": "2026-12-20", "to": "2026-12-31"},
                "metrics": [{"metric_key": "tasks.bugs_fixed", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.bugs_fixed", "period", entity_id=BOB).equals(value=1)
