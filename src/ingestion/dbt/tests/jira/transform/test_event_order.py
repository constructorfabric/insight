"""`event_order`: the one column that orders an issue's history.

Within one instant the entries are ranked, not the items: every field an entry
changed carries the entry's rank, because those changes happened together. The
from→to chains of all fields rank the entries, and the changelog id decides only
what no chain does.
"""

from __future__ import annotations

from typing import Any

from conftest import Scenario, case
from helpers import CREATED_AT, event, field, issue, item

ENTRY_RANK_SPAN = 100_000
MS_SPAN = 1_000_000

STATUS = "status"
ASSIGNEE = "assignee"
POINTS = "customfield_10001"
SEVERITY = "customfield_10003"

CATALOGUE = [
    field(STATUS, name="Status", schema_type="status"),
    field(ASSIGNEE, name="Assignee", schema_type="user"),
    field(POINTS, name="Story Points", schema_type="number"),
    field(
        SEVERITY,
        name="Severity",
        schema_type="option",
        schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:select",
    ),
]

STATUS_NAMES = {"1": "Open", "3": "In Progress", "5": "Resolved", "6": "Closed"}
USERS = {"alice-acct": "Alice Alpha", "bob-acct": "Bob Beta", "carol-acct": "Carol Gamma"}
SEVERITIES = {"9001": "Low", "9002": "High", "9003": "Critical"}

SAME_INSTANT = "2026-01-06T10:00:00"
BEFORE_CREATION = "2026-01-04T10:00:00"


def _status(frm: str, to: str) -> dict[str, Any]:
    return item(STATUS, frm=frm, frm_str=STATUS_NAMES[frm], to=to, to_str=STATUS_NAMES[to])


def _assignee(frm: str, to: str) -> dict[str, Any]:
    return item(ASSIGNEE, frm=frm, frm_str=USERS[frm], to=to, to_str=USERS[to])


def _points(frm: str, to: str) -> dict[str, Any]:
    return item(POINTS, frm=frm, frm_str=frm, to=to, to_str=to)


def _severity(frm: str, to: str) -> dict[str, Any]:
    return item(SEVERITY, frm=frm, frm_str=SEVERITIES[frm], to=to, to_str=SEVERITIES[to])


def _values(status: str, assignee: str | None = None, points: int | None = None) -> dict[str, Any]:
    values: dict[str, Any] = {STATUS: {"id": status, "name": STATUS_NAMES[status]}}
    if assignee is not None:
        values[ASSIGNEE] = {"accountId": assignee, "displayName": USERS[assignee]}
    if points is not None:
        values[POINTS] = points
    return values


def _changelog_orders(scenario: Scenario) -> dict[tuple[str, str], int]:
    return {
        (r["event_id"], r["field_id"]): r["event_order"] for r in scenario.journal() if r["event_kind"] == "changelog"
    }


def _entry_ranks(scenario: Scenario) -> dict[str, set[int]]:
    """Each entry's rank, as the set its rows carry — one value when shared."""
    ranks: dict[str, set[int]] = {}
    for (event_id, _field), order in _changelog_orders(scenario).items():
        ranks.setdefault(event_id, set()).add(order % ENTRY_RANK_SPAN)
    return ranks


@case(
    fields=CATALOGUE,
    issues=[issue("TST-1", fields=_values("6", assignee="bob-acct"))],
    events=[
        event("TST-1", 102, SAME_INSTANT, [_status("1", "3"), _assignee("alice-acct", "bob-acct")]),
        event("TST-1", 101, SAME_INSTANT, [_status("3", "6")]),
    ],
)
def test_every_field_of_one_entry_shares_its_rank(scenario: Scenario) -> None:
    """The status chain puts entry 102 first despite its larger id, and the
    assignee change it carries moves with it: one entry, one position."""
    orders = _changelog_orders(scenario)
    assert orders[("102", STATUS)] == orders[("102", ASSIGNEE)]
    assert _entry_ranks(scenario) == {"102": {0}, "101": {1}}
    assert scenario.round_trip_holds()


@case(
    fields=CATALOGUE,
    issues=[issue("TST-1", fields=_values("5", assignee="carol-acct"))],
    events=[
        event("TST-1", 302, SAME_INSTANT, [_status("1", "3")]),
        event("TST-1", 301, SAME_INSTANT, [_status("3", "5"), _assignee("alice-acct", "bob-acct")]),
        event("TST-1", 300, SAME_INSTANT, [_assignee("bob-acct", "carol-acct")]),
    ],
)
def test_chains_of_different_fields_link_entries_across_an_instant(scenario: Scenario) -> None:
    """Status orders 302 before 301 and assignee orders 301 before 300; no single
    field sees all three, and the ids run the other way."""
    assert _entry_ranks(scenario) == {"302": {0}, "301": {1}, "300": {2}}
    assert scenario.states(STATUS) == [["1"], ["3"], ["5"]]
    assert scenario.states(ASSIGNEE) == [["alice-acct"], ["bob-acct"], ["carol-acct"]]
    assert scenario.round_trip_holds()
    assert scenario.invariants_hold("assert_jira_same_instant_events_chain")


@case(
    fields=CATALOGUE,
    issues=[issue("TST-1", fields=_values("3"))],
    events=[event("TST-1", 101, BEFORE_CREATION, [_status("1", "3")])],
)
def test_the_initial_state_precedes_an_event_dated_before_the_creation(scenario: Scenario) -> None:
    """An imported history can date an entry before the issue was created. The
    initial row keeps the creation as its time and still sorts first."""
    rows = scenario.journal(field=STATUS)
    assert [(r["event_kind"], r["value_ids"]) for r in rows] == [("synthetic_initial", ["1"]), ("changelog", ["3"])]
    initial, change = rows
    assert initial["event_at"].startswith(CREATED_AT.replace("T", " "))
    assert initial["event_order"] // MS_SPAN == change["event_order"] // MS_SPAN
    marker = scenario.journal(field="created")
    assert marker[0]["event_order"] < initial["event_order"]


def test_contradicting_links_fall_back_only_where_they_contradict(scenario: Scenario) -> None:
    """Status says 101 comes before 100, severity says the opposite: those two
    entries fall back to their ids. The assignee link 100 → 50 and the points
    link 99 → 98 contradict nothing and still hold, though both run against the
    ids. The source contradicts itself, so the round trip cannot hold and the
    scenario keeps its own warehouse."""
    scenario.seed(
        fields=CATALOGUE,
        issues=[issue("TST-1", fields=_values("3", assignee="carol-acct", points=8))],
        events=[
            event(
                "TST-1",
                100,
                SAME_INSTANT,
                [_status("3", "6"), _severity("9001", "9002"), _assignee("alice-acct", "bob-acct")],
            ),
            event("TST-1", 101, SAME_INSTANT, [_status("1", "3"), _severity("9002", "9003")]),
            event("TST-1", 50, SAME_INSTANT, [_assignee("bob-acct", "carol-acct")]),
            event("TST-1", 99, SAME_INSTANT, [_points("3", "5")]),
            event("TST-1", 98, SAME_INSTANT, [_points("5", "8")]),
        ],
    )
    scenario.build()

    assert _entry_ranks(scenario) == {"99": {0}, "98": {1}, "100": {2}, "50": {3}, "101": {4}}
    assert scenario.states(ASSIGNEE) == [["alice-acct"], ["bob-acct"], ["carol-acct"]]
    assert scenario.states(POINTS) == [["3"], ["5"], ["8"]]
    assert not scenario.invariants_hold("assert_jira_same_instant_events_chain")


def test_identical_entries_of_one_instant_are_one_step_of_the_chain(scenario: Scenario) -> None:
    """Entries 200 and 201 record the same `1 -> 3` at one instant. Neither
    follows the other, but either order passes through the same states, so the
    chain holds."""
    scenario.seed(
        fields=CATALOGUE,
        issues=[issue("TST-1", fields=_values("5"))],
        events=[
            event("TST-1", 200, SAME_INSTANT, [_status("1", "3")]),
            event("TST-1", 201, SAME_INSTANT, [_status("1", "3")]),
            event("TST-1", 202, SAME_INSTANT, [_status("3", "5")]),
        ],
    )
    scenario.build()

    assert scenario.round_trip_holds()
    assert scenario.invariants_hold("assert_jira_same_instant_events_chain")


def test_an_entry_collapsed_to_one_event_is_one_step_of_the_chain(scenario: Scenario) -> None:
    """Entry 300 carries `3 -> ''` and `'' -> 5` of one field, which the
    journal collapses to `3 -> 5`; entry 301 continues `5 -> 8` at the same
    instant. The chain is read over the collapsed event, not its items."""
    scenario.seed(
        fields=CATALOGUE,
        issues=[issue("TST-1", fields=_values("1", points=8))],
        events=[
            event(
                "TST-1",
                300,
                SAME_INSTANT,
                [item(POINTS, frm="3", frm_str="3"), item(POINTS, to="5", to_str="5")],
            ),
            event("TST-1", 301, SAME_INSTANT, [_points("5", "8")]),
        ],
    )
    scenario.build()

    assert scenario.states(POINTS) == [["3"], ["5"], ["8"]]
    assert scenario.invariants_hold("assert_jira_same_instant_events_chain")
