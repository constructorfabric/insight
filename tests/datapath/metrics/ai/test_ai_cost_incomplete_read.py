"""AI cost: a Claude Team read that did not bring a day's whole roster never becomes that
day's state.

Bronze: per-user daily rows, plus one org envelope per read carrying the headcount that
read reported. Silver admits a read's per-user rows only when the people it returned equal
its own envelope's count; a read older than the first envelope has nothing to be judged
against and is admitted. Gold sums what silver admitted, so a rejected day serves nothing.
"""

from __future__ import annotations

from typing import Any

import pytest
from insight_datapath.metric_expect import MetricResponse, approx, one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_cost_incomplete_read"

ALICE = "alice@example.com"
BOB = "bob@example.com"

ALICE_DAYS = [
    ("2026-11-10", 0.5, "legacy read before the first envelope is admitted"),
    ("2026-11-15", 1.0, "read returning everyone its envelope counted is admitted"),
    ("2026-11-16", 0.0, "read short of its own headcount is rejected whole"),
    ("2026-11-17", 0.0, "read after the first envelope carrying none is rejected"),
    ("2026-11-18", 0.0, "the short read of a twice-read day is rejected"),
    ("2026-11-19", 0.0, "the first envelope's own read is not before itself"),
    ("2026-11-20", 0.0, "a day whose envelope counted people but stored none serves nothing"),
    ("2026-11-21", 0.0, "a day whose envelope counted nobody serves nothing"),
]

BOB_DAYS = [
    ("2026-11-15", 1.0, "read returning everyone its envelope counted is admitted"),
    ("2026-11-18", 0.75, "the sound read of a twice-read day is judged on its own envelope"),
]


def _ai_cost_by_day(spec: SpecRun, email: str) -> MetricResponse:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [email]},
                "period": {"from": "2026-11-01", "to": "2026-11-30"},
                "metrics": [
                    {
                        "metric_key": "ai.cost",
                        "views": [{"view": "period"}, {"view": "timeseries", "bucket": "day"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200
    return r


def _served_on(points: list[dict[str, Any]], day: str) -> float:
    return sum(float(p["value"] or 0) for p in points if p["bucket_start"] == day)


def test_only_legacy_and_complete_reads_reach_the_window(spec: SpecRun) -> None:
    """alice's window is 0.50 + 1.00; with the gate removed it would be 31.50."""
    r = _ai_cost_by_day(spec, ALICE)
    r.row("ai.cost", "period", entity_id=ALICE).equals(value=1.5)

    points = one(r.series("ai.cost"), entity_id=ALICE)["points"]
    for day, expected, rule in ALICE_DAYS:
        assert _served_on(points, day) == approx(expected), f"{rule}: {day} should serve {expected}"


def test_each_read_of_a_day_is_judged_against_its_own_envelope(spec: SpecRun) -> None:
    """Nov 18 was read twice; read 18 matched its count and read 19 did not. bob's sound
    read stands, so his window is 1.00 + 0.75."""
    r = _ai_cost_by_day(spec, BOB)
    r.row("ai.cost", "period", entity_id=BOB).equals(value=1.75)

    points = one(r.series("ai.cost"), entity_id=BOB)["points"]
    for day, expected, rule in BOB_DAYS:
        assert _served_on(points, day) == approx(expected), f"{rule}: {day} should serve {expected}"
