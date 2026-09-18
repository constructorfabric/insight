"""Codex: a ChatGPT Team read that did not bring a day's whole roster never becomes that
day's state.

Bronze: per-user daily rows, plus one envelope per read carrying the headcount that read
reported. Silver admits a read's per-user rows only when the people it returned equal its
own envelope's count; a read older than the first envelope has nothing to be judged
against and is admitted. Gold sums what silver admitted, so a rejected day serves nothing.

The last day also pins the meaning of `credits`: it is on-demand usage, not total Codex
consumption, so a day of real thread activity that consumed no credits is ordinary work
and has to serve like any other.
"""

from __future__ import annotations

from typing import Any

import pytest
from insight_datapath.metric_expect import MetricResponse, approx, one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "ai_codex_incomplete_read"

ALICE = "alice@example.com"
BOB = "bob@example.com"

ALICE_DAYS = [
    ("2026-12-01", 1.0, "legacy read before the first envelope is admitted"),
    ("2026-12-02", 2.0, "read returning everyone its envelope counted is admitted"),
    ("2026-12-03", 0.0, "read short of its own headcount is rejected whole"),
    ("2026-12-04", 0.0, "read after the first envelope carrying none is rejected"),
    ("2026-12-05", 3.0, "activity that consumed no credits still serves"),
]

BOB_DAYS = [
    ("2026-12-02", 1.0, "read returning everyone its envelope counted is admitted"),
]


def _dev_conversations_by_day(spec: SpecRun, email: str) -> MetricResponse:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [email]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {
                        "metric_key": "ai.dev_conversations",
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
    """alice's window is 1 + 2 + 3; with the gate removed it would be 18."""
    r = _dev_conversations_by_day(spec, ALICE)
    r.row("ai.dev_conversations", "period", entity_id=ALICE).equals(value=6)

    points = one(r.series("ai.dev_conversations"), entity_id=ALICE)["points"]
    for day, expected, rule in ALICE_DAYS:
        assert _served_on(points, day) == approx(expected), f"{rule}: {day} should serve {expected}"


def test_a_short_read_takes_the_whole_day_with_it(spec: SpecRun) -> None:
    """bob was the person Dec 03's read lost. Rejecting only the missing person would
    leave alice's row standing and the day would serve a number nobody can defend; the
    read is rejected whole, so bob's window is his single complete day."""
    r = _dev_conversations_by_day(spec, BOB)
    r.row("ai.dev_conversations", "period", entity_id=BOB).equals(value=1)

    points = one(r.series("ai.dev_conversations"), entity_id=BOB)["points"]
    for day, expected, rule in BOB_DAYS:
        assert _served_on(points, day) == approx(expected), f"{rule}: {day} should serve {expected}"
    assert _served_on(points, "2026-12-03") == approx(0.0), (
        "the short read must not serve the person it did return either"
    )
