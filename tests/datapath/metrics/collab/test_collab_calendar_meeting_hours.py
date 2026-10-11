"""Calendar meeting hours count the time booked by accepted meetings.

Bronze: one calendar event per person per occurrence, a later read replacing an
earlier one. Staging keeps accepted meetings and meetings organized with someone
else invited, shown as busy, not cancelled, not all-day and at most eight hours;
overlaps count once, a meeting across midnight is split between the days, and
only days inside the read window count. A later read that moves an event away
leaves its old day at zero. Gold sums the hours.
"""

from __future__ import annotations

import pytest
from insight_datapath.ch_seeder import CHSeeder
from insight_datapath.clickhouse import client
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.instance import InstanceConfig
from insight_datapath.metric_expect import MetricResponse, approx
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_calendar_meeting_hours"

ALICE = "alice@example.com"
METRIC = "collab.calendar_meeting_hours"
REBUILT_MODELS = "m365__collab_calendar_activity+"

MOVED_EVENT_FIELDS = {
    "_airbyte_raw_id": "00000000-0000-0000-0000-000000000016",
    "_airbyte_extracted_at": "2027-01-01T02:00:00",
    "start_time": "2026-12-24T08:00:00.0000000",
    "end_time": "2026-12-24T16:00:00.0000000",
}


def _period(day_from: str, day_to: str) -> dict[str, object]:
    return {
        "url": "/v1/metric-results",
        "method": "POST",
        "body": {
            "entity": {"type": "person", "ids": [ALICE]},
            "period": {"from": day_from, "to": day_to},
            "metrics": [{"metric_key": METRIC, "views": [{"view": "period"}]}],
        },
    }


def _hours(spec: SpecRun, day_from: str, day_to: str) -> MetricResponse:
    r = spec.call(_period(day_from, day_to))
    assert r.status == 200, f"should answer 200 for {day_from}..{day_to}"
    return r


def _assert_hours(r: MetricResponse, day_from: str, day_to: str, expected: float | None) -> None:
    row = r.row(METRIC, "period", entity_id=ALICE)
    if expected is None:
        row.equals(value=None)
        return
    row.check(
        "value",
        lambda v: v is not None and float(v) == approx(expected),
        f"{METRIC} over {day_from}..{day_to} should be {expected}",
    )


@pytest.mark.parametrize(
    ("day_from", "day_to", "expected"),
    [
        pytest.param("2026-12-03", "2026-12-03", None, id="day-before-the-window-stays-empty"),
        pytest.param(
            "2026-12-04",
            "2026-12-04",
            1,
            id="first-window-day-keeps-its-part-of-a-crossing-meeting",
        ),
        pytest.param(
            "2026-12-07", "2026-12-07", 1.5, id="overlaps-count-once-and-unaccepted-adds-nothing"
        ),
        pytest.param(
            "2026-12-08",
            "2026-12-08",
            1,
            id="organized-alone-is-no-meeting-and-a-resent-event-counts-once",
        ),
        pytest.param(
            "2026-12-09", "2026-12-09", 1, id="meeting-across-midnight-counts-its-first-day-part"
        ),
        pytest.param(
            "2026-12-10", "2026-12-10", 1, id="meeting-across-midnight-counts-its-second-day-part"
        ),
        pytest.param(
            "2026-12-11", "2026-12-11", 0, id="event-longer-than-eight-hours-adds-nothing"
        ),
        pytest.param(
            "2026-12-14", "2026-12-14", 0, id="meeting-read-again-as-cancelled-adds-nothing"
        ),
        pytest.param("2026-12-15", "2026-12-15", 3.5, id="meeting-inside-another-counts-once"),
        pytest.param("2026-12-16", "2026-12-16", 8, id="eight-hour-meeting-counts"),
        pytest.param(
            "2026-12-17", "2026-12-17", 1, id="meeting-ending-at-midnight-stays-on-its-day"
        ),
        pytest.param(
            "2026-12-18", "2026-12-18", None, id="meeting-ending-at-midnight-adds-no-next-day"
        ),
        pytest.param(
            "2026-12-21", "2026-12-21", 0, id="organized-with-unknown-invitees-is-no-meeting"
        ),
        pytest.param(
            "2026-12-22", "2026-12-22", 1, id="sign-in-name-case-does-not-split-the-person"
        ),
        pytest.param(
            "2026-12-30", "2026-12-30", 1, id="last-window-day-keeps-its-part-of-a-crossing-meeting"
        ),
        pytest.param("2026-12-31", "2026-12-31", None, id="day-after-the-window-stays-empty"),
        pytest.param("2026-12-01", "2026-12-31", 20, id="month-total"),
    ],
)
def test_calendar_meeting_hours_count_accepted_booked_time(
    spec: SpecRun, day_from: str, day_to: str, expected: float | None
) -> None:
    _assert_hours(_hours(spec, day_from, day_to), day_from, day_to, expected)


def test_a_moved_event_empties_its_old_day(
    spec: SpecRun, instance_cfg: InstanceConfig, ch_seeder: CHSeeder, dbt_runner: DbtRunner
) -> None:
    _assert_hours(_hours(spec, "2026-12-16", "2026-12-16"), "2026-12-16", "2026-12-16", 8)

    with client(instance_cfg) as connection:
        rows = connection.query(
            "SELECT * FROM bronze_m365.calendar_events FINAL WHERE unique_key = 'cal-alice-e16'"
        )
        event = next(rows.named_results())
    ch_seeder.seed_records("bronze_m365", "calendar_events", [{**event, **MOVED_EVENT_FIELDS}])
    dbt_runner.run(REBUILT_MODELS)

    _assert_hours(_hours(spec, "2026-12-16", "2026-12-16"), "2026-12-16", "2026-12-16", 0)
    _assert_hours(_hours(spec, "2026-12-24", "2026-12-24"), "2026-12-24", "2026-12-24", 8)
