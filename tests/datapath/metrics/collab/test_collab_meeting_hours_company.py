"""Zoom meeting time counts only while someone else was in the meeting.

Bronze: the daily Teams activity report and Zoom participant sessions. Staging
collapses Zoom rows re-sent by a sync, counts a person's session time from the
first other arrival to the last other departure, and treats a session with nobody
else as no meeting. Gold sums meeting hours across Teams and Zoom.
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


def _period(person: str, metric_key: str) -> dict[str, object]:
    return {
        "url": "/v1/metric-results",
        "method": "POST",
        "body": {
            "entity": {"type": "person", "ids": [person]},
            "period": {"from": "2026-12-01", "to": "2026-12-31"},
            "metrics": [{"metric_key": metric_key, "views": [{"view": "period"}]}],
        },
    }


@pytest.mark.parametrize(
    ("person", "metric_key", "expected"),
    [
        pytest.param(
            ALICE, "collab.meeting_hours", 1.5, id="teams-hour-plus-zoom-time-with-company"
        ),
        pytest.param(BOB, "collab.meeting_hours", 0.5, id="time-overlapping-with-others"),
        pytest.param(
            CAROL, "collab.meeting_hours", 0.1, id="room-left-open-adds-only-the-company-time"
        ),
        pytest.param(CAROL, "collab.meetings_count", 1, id="room-with-a-visitor-is-one-meeting"),
        pytest.param(ERIN, "collab.meeting_hours", 0, id="solo-session-adds-no-time"),
        pytest.param(ERIN, "collab.meetings_count", 0, id="solo-session-is-no-meeting"),
    ],
)
def test_meeting_time_counts_only_with_company(
    spec: SpecRun, person: str, metric_key: str, expected: float
) -> None:
    r = spec.call(_period(person, metric_key))
    assert r.status == 200, f"should answer 200 for {person} {metric_key}"
    r.row(metric_key, "period", entity_id=person).check(
        "value",
        lambda v: v is not None and float(v) == approx(expected),
        f"{metric_key} for {person} should be {expected}",
    )
