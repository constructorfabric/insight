"""AI active days, served per person over a month with the department as peers.

Bronze: Cursor daily per-developer usage, one row marking a Cursor user as active
on a day. Silver: deduped to one row per developer/day. Gold serves an "active"
metric: value 1 for the requested person, with the peer view sized by the
department's active Cursor members. alice, bob and carol are all active, so the
pool holds three.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_active_days"

ALICE = "alice@example.com"


def test_ai_active_days(spec: SpecRun) -> None:
    """alice, bob and carol are each active on Jan 05, so alice's value is 1 and the
    department pool holds three."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [
                    {
                        "metric_key": "ai.active_days",
                        "views": [
                            {"view": "period"},
                            {"view": "peer"},
                            {"view": "timeseries", "bucket": "day"},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("ai.active_days", "period", entity_id=ALICE).equals(value=1)
    r.row("ai.active_days", "peer", entity_id=ALICE).equals(
        target_value=1, p25=None, median=None, p75=None, min=None, max=None, n=3
    )
    active = one(r.series("ai.active_days"), entity_id=ALICE)["points"]
    assert float(one(active, bucket_start="2026-01-05")["value"]) == 1.0
