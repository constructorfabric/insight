"""Teams and Zoom meetings add up under one definition, and Zoom counts each meeting once.

Bronze: the daily Teams activity report and Zoom meetings with participant
sessions. A Teams one-to-one call counts as a meeting, as a two-person Zoom meeting
does. Staging stitches the sessions of one Zoom meeting that restarts within five minutes, and dates a Zoom meeting
by the UTC day the person joined it. Gold sums meetings attended across both tools
with a tool dimension and a kind dimension: Teams tells scheduled meetings, ad hoc
meetings and calls apart, Zoom is one kind.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import MetricResponse
from insight_datapath.spec_runner import SpecRun

pytestmark = [pytest.mark.fixture, pytest.mark.shared_dataset]

SPEC = "collab_meetings_count_tools"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"
DAVE = "dave@example.com"
ERIN = "erin@example.com"


def _request(
    person: str, date_from: str, date_to: str, dimension: str = "tool"
) -> dict[str, object]:
    return {
        "url": "/v1/metric-results",
        "method": "POST",
        "body": {
            "entity": {"type": "person", "ids": [person]},
            "period": {"from": date_from, "to": date_to},
            "metrics": [
                {
                    "metric_key": "collab.meetings_count",
                    "views": [
                        {"view": "period"},
                        {"view": "breakdown", "dimensions": [dimension]},
                    ],
                }
            ],
        },
    }


def _expect_kinds(r: MetricResponse, person: str, expected: dict[str, int]) -> None:
    for kind, value in expected.items():
        r.row(
            "collab.meetings_count",
            "breakdown",
            entity_id=person,
            dimensions={"key": "kind", "value": kind},
        ).equals(value=value)
    served = len(r.breakdown("collab.meetings_count"))
    assert served == len(expected), f"should serve only the counted kinds for {person}: {served}"


def test_teams_calls_count_and_tools_add_up_and_split_by_tool_and_kind(spec: SpecRun) -> None:
    r = spec.call(_request(ALICE, "2026-12-01", "2026-12-31"))
    assert r.status == 200, f"should answer 200 for {ALICE} in December"
    r.row("collab.meetings_count", "period", entity_id=ALICE).equals(value=5)
    for tool, expected in (("m365", 3), ("zoom", 2)):
        r.row(
            "collab.meetings_count",
            "breakdown",
            entity_id=ALICE,
            dimensions={"key": "tool", "value": tool},
        ).equals(value=expected)

    r = spec.call(_request(ALICE, "2026-12-01", "2026-12-31", dimension="kind"))
    assert r.status == 200, f"should answer 200 for {ALICE} by kind"
    _expect_kinds(r, ALICE, {"teams_scheduled": 1, "teams_adhoc": 1, "teams_call": 1, "zoom": 2})


@pytest.mark.parametrize(
    ("person", "expected_total", "expected_kinds"),
    [
        pytest.param(
            DAVE,
            3,
            {"teams_scheduled": 1, "teams_adhoc": 1, "teams_other": 1},
            id="a-meeting-in-neither-split-keeps-its-own-kind",
        ),
        pytest.param(
            ERIN,
            1,
            {"teams_scheduled": 1},
            id="splits-beyond-the-total-are-capped-by-it",
        ),
    ],
)
def test_teams_kinds_add_up_to_the_reported_total(
    spec: SpecRun, person: str, expected_total: int, expected_kinds: dict[str, int]
) -> None:
    r = spec.call(_request(person, "2026-12-12", "2026-12-12", dimension="kind"))
    assert r.status == 200, f"should answer 200 for {person} on Dec 12"
    r.row("collab.meetings_count", "period", entity_id=person).equals(value=expected_total)
    _expect_kinds(r, person, expected_kinds)


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
