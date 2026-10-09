"""Teams and Zoom organizers add up.

Bronze: the daily Teams activity report and Zoom meetings with their host and
participant sessions. Staging counts a Zoom meeting for its host on the UTC day it
started, only if someone besides the host attended, and also when the host never
joined. Gold sums meetings organized across both tools with a tool dimension.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import one
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_meetings_organized_tools"

ALICE = "alice@example.com"
CAROL = "carol@example.com"
DAVE = "dave@example.com"


def _request(person: str, views: list[dict[str, object]]) -> dict[str, object]:
    return {
        "url": "/v1/metric-results",
        "method": "POST",
        "body": {
            "entity": {"type": "person", "ids": [person]},
            "period": {"from": "2026-12-01", "to": "2026-12-31"},
            "metrics": [{"metric_key": "collab.meetings_organized", "views": views}],
        },
    }


def test_teams_and_zoom_organizers_add_up_and_split_by_tool(spec: SpecRun) -> None:
    r = spec.call(
        _request(ALICE, [{"view": "period"}, {"view": "breakdown", "dimensions": ["tool"]}])
    )
    assert r.status == 200, f"should answer 200 for {ALICE}"
    r.row("collab.meetings_organized", "period", entity_id=ALICE).equals(value=3)
    by_tool = r.breakdown("collab.meetings_organized")
    for tool, expected in (("m365", 2), ("zoom", 1)):
        row = one(by_tool, entity_id=ALICE, dimensions={"key": "tool", "value": tool})
        assert float(row["value"]) == expected, f"should count {expected} for {tool}: {row}"


@pytest.mark.parametrize(
    ("person", "expected"),
    [
        pytest.param(CAROL, 0, id="meeting-nobody-else-joined-is-not-organized"),
        pytest.param(DAVE, 1, id="host-who-never-joined-still-organized-it"),
    ],
)
def test_zoom_organizer_needs_an_attendee_besides_the_host(
    spec: SpecRun, person: str, expected: int
) -> None:
    r = spec.call(_request(person, [{"view": "period"}]))
    assert r.status == 200, f"should answer 200 for {person}"
    r.row("collab.meetings_organized", "period", entity_id=person).equals(value=expected)
