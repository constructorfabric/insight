"""Teams and Zoom meetings add up under one definition, and Zoom counts each meeting once.

Bronze: the daily Teams activity report and Zoom meetings with participant
sessions. A Teams one-to-one call counts as a meeting, as a two-person Zoom meeting
does. Staging stitches the sessions of one Zoom meeting that restarts within five minutes, and dates a Zoom meeting
by the UTC day the person joined it. Gold sums meetings attended across both tools
with a tool dimension.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_meetings_count_tools"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"


def _request(person: str, date_from: str, date_to: str) -> dict[str, object]:
    return {
        "url": "/v1/metric-results",
        "method": "POST",
        "body": {
            "entity": {"type": "person", "ids": [person]},
            "period": {"from": date_from, "to": date_to},
            "metrics": [
                {
                    "metric_key": "collab.meetings_count",
                    "views": [{"view": "period"}, {"view": "breakdown", "dimensions": ["tool"]}],
                }
            ],
        },
    }


def test_teams_calls_count_and_tools_add_up_and_split_by_tool(spec: SpecRun) -> None:
    r = spec.call(_request(ALICE, "2026-12-01", "2026-12-31"))
    assert r.status == 200, f"should answer 200 for {ALICE} in December"
    r.row("collab.meetings_count", "period", entity_id=ALICE).equals(value=5)
    by_tool = r.breakdown("collab.meetings_count")
    for tool, expected in (("m365", 3), ("zoom", 2)):
        row = one(by_tool, entity_id=ALICE, dimensions={"key": "tool", "value": tool})
        assert float(row["value"]) == expected, f"should count {expected} for {tool}: {row}"


@pytest.mark.parametrize(
    ("person", "date_from", "date_to", "expected"),
    [
        pytest.param(
            BOB, "2026-12-01", "2026-12-31", 2, id="restart-after-five-minutes-is-two-meetings"
        ),
        pytest.param(CAROL, "2026-12-31", "2026-12-31", 1, id="join-day-decides-the-date"),
        pytest.param(
            CAROL, "2027-01-01", "2027-01-01", None, id="meeting-is-not-counted-on-the-next-day"
        ),
    ],
)
def test_zoom_meetings_are_counted_once_on_the_join_day(
    spec: SpecRun, person: str, date_from: str, date_to: str, expected: int | None
) -> None:
    r = spec.call(_request(person, date_from, date_to))
    assert r.status == 200, f"should answer 200 for {person} {date_from}..{date_to}"
    r.row("collab.meetings_count", "period", entity_id=person).equals(value=expected)
