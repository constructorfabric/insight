"""Zoom meeting time counts only while someone else was in the meeting.

Bronze: the daily Teams activity report and Zoom participant sessions. Staging
collapses Zoom rows re-sent by a sync, counts a person's session only while at
least one other attendee is in it, and treats a session with nobody else as no
meeting. Gold sums meeting hours across Teams and Zoom.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import approx
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_meeting_hours_company"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"
ERIN = "erin@example.com"
HEIDI = "heidi@example.com"

DECEMBER = ("2026-12-01", "2026-12-31")
PAUSE_DAY = ("2026-12-14", "2026-12-14")
NO_OVERLAP_DAY = ("2026-12-15", "2026-12-15")


def _period(person: str, metric_key: str, day_from: str, day_to: str) -> dict[str, object]:
    return {
        "url": "/v1/metric-results",
        "method": "POST",
        "body": {
            "entity": {"type": "person", "ids": [person]},
            "period": {"from": day_from, "to": day_to},
            "metrics": [{"metric_key": metric_key, "views": [{"view": "period"}]}],
        },
    }


@pytest.mark.parametrize(
    ("person", "days", "metric_key", "expected"),
    [
        pytest.param(
            ALICE,
            DECEMBER,
            "collab.meeting_hours",
            1.5,
            id="teams-hour-plus-zoom-time-with-company",
        ),
        pytest.param(BOB, DECEMBER, "collab.meeting_hours", 0.5, id="time-overlapping-with-others"),
        pytest.param(
            CAROL,
            DECEMBER,
            "collab.meeting_hours",
            0.1,
            id="room-left-open-adds-only-the-company-time",
        ),
        pytest.param(
            CAROL, DECEMBER, "collab.meetings_count", 1, id="room-with-a-visitor-is-one-meeting"
        ),
        pytest.param(ERIN, DECEMBER, "collab.meeting_hours", 0, id="solo-session-adds-no-time"),
        pytest.param(ERIN, DECEMBER, "collab.meetings_count", 0, id="solo-session-is-no-meeting"),
        pytest.param(
            HEIDI, PAUSE_DAY, "collab.meeting_hours", 2, id="pause-between-visitors-adds-nothing"
        ),
        pytest.param(
            HEIDI, PAUSE_DAY, "collab.meetings_count", 1, id="visited-room-is-one-meeting"
        ),
        pytest.param(
            HEIDI,
            NO_OVERLAP_DAY,
            "collab.meeting_hours",
            0,
            id="no-overlap-with-anyone-adds-no-time",
        ),
        pytest.param(
            HEIDI,
            NO_OVERLAP_DAY,
            "collab.meetings_count",
            0,
            id="no-overlap-with-anyone-is-no-meeting",
        ),
    ],
)
def test_meeting_time_counts_only_with_company(
    spec: SpecRun, person: str, days: tuple[str, str], metric_key: str, expected: float
) -> None:
    r = spec.call(_period(person, metric_key, *days))
    assert r.status == 200, f"should answer 200 for {person} {metric_key} {days}"
    r.row(metric_key, "period", entity_id=person).check(
        "value",
        lambda v: v is not None and float(v) == approx(expected),
        f"{metric_key} for {person} over {days} should be {expected}",
    )
