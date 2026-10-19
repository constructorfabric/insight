"""The metrics derived from a person's active days, across tools and addresses.

An active day has a deliberate action under any of the person's addresses. Breadth
counts the modalities behind those days, a meeting-free day is an active day without
meeting time under any address, and focus is 100 minus meeting hours as a share of
working hours over the active days, with one working day per day however many
addresses were active and a day's meetings counted up to its working hours. Both
leave out addresses no meeting source ever reported, whose meeting time is unknown.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import MetricResponse, approx, one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_derived_days"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"
DAVE = "dave@example.com"

DECEMBER = ("2026-12-01", "2026-12-31")
EMPTY_WINDOW = ("2026-11-01", "2026-11-30")

FOCUS = "collab.focus_time_pct"
DERIVED_METRICS = (
    "collab.active_days",
    "collab.breadth",
    "collab.meeting_free_days",
    "collab.focus_time_pct",
)

EXPECTED = {
    ALICE: {
        "collab.active_days": 5,
        "collab.breadth": 4,
        "collab.meeting_free_days": 2,
        "collab.focus_time_pct": 65.0,
    },
    BOB: {
        "collab.active_days": 2,
        "collab.breadth": 2,
        "collab.meeting_free_days": 1,
        "collab.focus_time_pct": 81.25,
    },
    CAROL: {
        "collab.active_days": 1,
        "collab.breadth": 1,
        "collab.meeting_free_days": 1,
        "collab.focus_time_pct": 100.0,
    },
    DAVE: {
        "collab.active_days": 1,
        "collab.breadth": 1,
        "collab.meeting_free_days": None,
        "collab.focus_time_pct": None,
    },
}


def _call(
    spec: SpecRun, person: str, window: tuple[str, str], keys: tuple[str, ...], view: dict
) -> MetricResponse:
    return spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [person]},
                "period": {"from": window[0], "to": window[1]},
                "metrics": [{"metric_key": key, "views": [view]} for key in keys],
            },
        }
    )


@pytest.mark.parametrize("person", [ALICE, BOB, CAROL, DAVE])
def test_derived_metrics_count_each_active_day_once(spec: SpecRun, person: str) -> None:
    """Each person's four derived metrics follow the active days of the cast."""
    r = _call(spec, person, DECEMBER, DERIVED_METRICS, {"view": "period"})
    assert r.status == 200
    for key, value in EXPECTED[person].items():
        r.row(key, "period", entity_id=person).equals(value=value)


@pytest.mark.parametrize(
    ("day", "focus"),
    [
        ("2026-12-14", 100.0),
        ("2026-12-15", 75.0),
        ("2026-12-16", 50.0),
        ("2026-12-17", 0.0),
        ("2026-12-19", 100.0),
    ],
)
def test_focus_shares_one_working_day_across_addresses(
    spec: SpecRun, day: str, focus: float
) -> None:
    """Wednesday's Teams chat and Zoom meeting under two addresses share one working
    day, and Thursday's ten meeting hours count as the whole eight-hour day."""
    r = _call(spec, ALICE, DECEMBER, (FOCUS,), {"view": "timeseries", "bucket": "day"})
    assert r.status == 200
    points = one(r.series(FOCUS), entity_id=ALICE)["points"]
    assert float(one(points, bucket_start=day)["value"]) == approx(focus), (
        f"alice's focus on {day} should be {focus}"
    )


def test_empty_window_serves_null(spec: SpecRun) -> None:
    """A window with no activity serves a null for every derived metric."""
    r = _call(spec, ALICE, EMPTY_WINDOW, DERIVED_METRICS, {"view": "period"})
    assert r.status == 200
    for key in DERIVED_METRICS:
        r.row(key, "period", entity_id=ALICE).equals(value=None)
