"""GitHub task resolution time and reopen rate, read from a folded state history.

A GitHub timeline records changes, so an issue's creation is not in it: the staging
model synthesises a creation marker from the snapshot and gold reads it as `created_at`.
Both issues are created on 2026-03-01 and their first timeline event is weeks later, so
resolution time counts from creation rather than from the first recorded event. The
second issue closes, reopens and closes again; the reopen rate is folded out of that
history, which the snapshot alone cannot supply.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "github_tasks_lifecycle"

CAROL = "carol@example.com"


def test_resolution_counted_from_the_synthesised_creation_marker(spec: SpecRun) -> None:
    """2026-03-01 to 2026-03-25 is 24 days; a model reading the first recorded event
    instead of the creation marker would report about zero here."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-03-20", "to": "2026-03-31"},
                "metrics": [
                    {"metric_key": "tasks.closed", "views": [{"view": "period"}]},
                    {"metric_key": "tasks.resolution_time", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("tasks.closed", "period", entity_id=CAROL).equals(value=2)
    resolution = float(one(r.rows("tasks.resolution_time", "period"), entity_id=CAROL)["value"])
    assert 23.5 < resolution < 24.5


def test_reopen_rate_folded_out_of_the_state_history(spec: SpecRun) -> None:
    """Three closures across the two issues, one on the 5th and two on the 25th, and
    exactly one of them undone within a fortnight."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-03-01", "to": "2026-03-31"},
                "metrics": [{"metric_key": "tasks.reopen_rate", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200

    reopen_rate = float(one(r.rows("tasks.reopen_rate", "period"), entity_id=CAROL)["value"])
    assert 33.0 < reopen_rate < 34.0
