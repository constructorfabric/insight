"""Order, duplicates and repeated syncs.

Bronze is append-only and Airbyte re-emits, so the same changelog arrives many
times and the same issue arrives in several versions. None of that may change
the journal: `unique_key` is a pure function of content, so two runs over the
same bronze produce byte-identical keys and ReplacingMergeTree collapses them.

Order within one instant is where the journal can go wrong without losing a
row: an inverted element-wise pair changes the resulting set, and an inverted
chain of self-describing transitions ends the field one step short.
"""

from __future__ import annotations

from typing import Any

from conftest import Scenario, case
from helpers import CREATED_AT, LATER_SYNC, event, field, issue, item

COMPONENTS = "components"
POINTS = "customfield_10001"
COMPONENTS_FIELD = field(COMPONENTS, name="Components", schema_type="array", schema_items="component")
POINTS_FIELD = field(POINTS, name="Story Points", schema_type="number")

SAME_INSTANT = "2026-01-06T10:00:00"


@case(
    fields=[COMPONENTS_FIELD],
    issues=[issue("TST-1", fields={COMPONENTS: []})],
    events=[
        event("TST-1", 99, SAME_INSTANT, [item(COMPONENTS, to="501", to_str="api")]),
        event("TST-1", 101, SAME_INSTANT, [item(COMPONENTS, frm="501", frm_str="api")]),
    ],
)
def test_events_sharing_an_instant_are_ordered_by_changelog_id(scenario: Scenario) -> None:
    """Two events in the same second, one adding an element and one removing it.

    Their relative order decides whether the field ends up holding the element,
    so the tie-break has to follow Jira's own monotonic changelog id — and
    numerically, not as text: ids cross a digit-count boundary all the time, and
    `'101' < '99'` as strings.
    """
    assert scenario.states(COMPONENTS) == [[], ["501"], []]
    assert scenario.round_trip_holds()


POINTS_EVENT = event("TST-1", 101, SAME_INSTANT, [item(POINTS, frm="3", frm_str="3", to="5", to_str="5")])
REPEATED_ITEM = item(POINTS, frm="3", frm_str="3", to="5", to_str="5")


@case(
    fields=[POINTS_FIELD],
    issues=[issue("TST-1", fields={POINTS: 5})],
    events=[POINTS_EVENT, dict(POINTS_EVENT, _airbyte_extracted_at=LATER_SYNC)],
)
def test_the_same_changelog_re_emitted_produces_one_row(scenario: Scenario) -> None:
    """Airbyte appends, so a changelog the connector has seen before arrives
    again on the next sync. Two rows in bronze, one event in the journal."""
    assert scenario.states(POINTS) == [["3"], ["5"]]
    assert scenario.round_trip_holds()


@case(
    fields=[POINTS_FIELD],
    issues=[issue("TST-1", fields={POINTS: 5})],
    events=[event("TST-1", 101, SAME_INSTANT, [REPEATED_ITEM, REPEATED_ITEM])],
)
def test_an_item_repeated_inside_one_changelog_produces_one_row(scenario: Scenario) -> None:
    """Jira sometimes puts the same (field, from, to) twice in one entry's
    items array. Both would carry the same event id, so the second is not a
    second event."""
    assert scenario.states(POINTS) == [["3"], ["5"]]


def test_building_twice_changes_nothing(scenario: Scenario) -> None:
    """Idempotence is what lets the model be rebuilt or re-run at will, and it
    rests on `unique_key` being a pure function of content — no clock, no run
    id, no row order."""
    scenario.seed(
        fields=[POINTS_FIELD, COMPONENTS_FIELD],
        issues=[issue("TST-1", fields={POINTS: 5, COMPONENTS: [{"id": "501", "name": "api"}]})],
        events=[event("TST-1", 101, SAME_INSTANT, [item(POINTS, frm="3", frm_str="3", to="5", to_str="5")])],
    )
    scenario.build()
    first = [(r["field_id"], r["event_id"], r["value_ids"]) for r in scenario.journal()]

    scenario.build()
    second = [(r["field_id"], r["event_id"], r["value_ids"]) for r in scenario.journal()]

    assert first == second


@case(
    fields=[POINTS_FIELD],
    issues=[issue("TST-1", fields={POINTS: 3}), issue("TST-1", fields={POINTS: 8}, extracted_at=LATER_SYNC)],
)
def test_a_newer_issue_version_supersedes_the_older(scenario: Scenario) -> None:
    """The issue's current value comes from ONE chosen bronze row. Resolving it
    per column instead lets two syncs mix, which is how an issue ends up with a
    status from one version and a value from another."""
    assert scenario.states(POINTS) == [["8"]]
    assert scenario.round_trip_holds()


@case(
    fields=[field("project", name="Project", schema_type="project")],
    issues=[issue("TST-1", fields={"project": {"id": "902", "key": "NEW", "name": "New Project"}})],
    events=[
        event(
            "TST-1",
            101,
            SAME_INSTANT,
            [item("project", frm="901", frm_str="Old Project", to="902", to_str="New Project")],
        )
    ],
)
def test_an_issue_moved_between_projects_records_the_move(scenario: Scenario) -> None:
    """`project` is an ordinary object field with real changelog traffic, not a
    container: an issue moved between projects has to show it."""
    rows = scenario.journal(field="project")
    assert [r["value_ids"] for r in rows] == [["901"], ["902"]]
    # `name` must win over `key` in the display probe, or the two sides of the
    # pipeline stop agreeing on what a project object is called.
    assert rows[-1]["value_displays"] == ["New Project"]
    assert scenario.round_trip_holds()


@case(
    fields=[
        field(
            "customfield_11300",
            name="Epic Link",
            schema_type="any",
            schema_custom="com.pyxis.greenhopper.jira:gh-epic-link",
        )
    ],
    issues=[issue("TST-1", fields={"customfield_11300": "TST-9"})],
    events=[event("TST-1", 101, CREATED_AT, [item("customfield_11300", to="4242", to_str="TST-9")])],
)
def test_an_event_on_the_creation_instant_still_wins(scenario: Scenario) -> None:
    """An issue whose first event happened at its own creation.

    Both rows then carry the same `event_at`, and `_seq` sorts them the wrong
    way round: it is 0 for the changelog row and 1..N for the initial one. A
    reader ordering by `(event_at, _seq)` — which the contract calls a total
    order — reads the field as still empty. The kind is what breaks the tie,
    because an initial row is by definition the state before any event.
    """
    assert scenario.states("customfield_11300") == [[], ["TST-9"]]
    assert scenario.round_trip_holds()


STATUS = "status"
STATUS_FIELD = field(STATUS, name="Status", schema_type="status")
STATUS_NAMES = {"1": "Open", "3": "In Progress", "5": "Resolved", "6": "Closed", "7": "Reopened"}
EARLIER = "2026-01-06T09:00:00"
LATER = "2026-01-06T11:00:00"


def _transition(changelog_id: int, at: str, frm: str, to: str) -> dict[str, Any]:
    return event(
        "TST-1", changelog_id, at, [item(STATUS, frm=frm, frm_str=STATUS_NAMES[frm], to=to, to_str=STATUS_NAMES[to])]
    )


def _status_now(status_id: str) -> dict[str, Any]:
    return issue("TST-1", fields={STATUS: {"id": status_id, "name": STATUS_NAMES[status_id]}})


def _changelog_seq(scenario: Scenario) -> dict[str, int]:
    return {r["event_id"]: r["_seq"] for r in scenario.journal(field=STATUS) if r["event_kind"] == "changelog"}


@case(
    fields=[STATUS_FIELD],
    issues=[_status_now("6")],
    events=[
        _transition(100, EARLIER, "1", "3"),
        _transition(102, SAME_INSTANT, "3", "5"),
        _transition(101, SAME_INSTANT, "5", "6"),
    ],
)
def test_a_chain_sharing_an_instant_ends_on_its_last_step_whatever_the_ids(scenario: Scenario) -> None:
    """An imported history can stamp two transitions with one second and give
    the later step the smaller changelog id. The from→to chain decides."""
    assert scenario.states(STATUS) == [["1"], ["3"], ["5"], ["6"]]
    assert _changelog_seq(scenario) == {"100": 0, "102": 0, "101": 1}
    assert scenario.round_trip_holds()


@case(
    fields=[STATUS_FIELD],
    issues=[_status_now("6")],
    events=[
        _transition(202, SAME_INSTANT, "1", "3"),
        _transition(201, SAME_INSTANT, "3", "5"),
        _transition(203, LATER, "5", "6"),
    ],
)
def test_the_initial_value_is_the_before_side_of_the_chain_head(scenario: Scenario) -> None:
    """When the tie is at the first change, the value at creation is the
    `before` side of the chain's head, not of the smaller changelog id."""
    initial = [r["value_ids"] for r in scenario.journal(field=STATUS) if r["event_kind"] == "synthetic_initial"]
    assert initial == [["1"]]
    assert scenario.states(STATUS) == [["1"], ["3"], ["5"], ["6"]]
    assert scenario.round_trip_holds()


@case(
    fields=[STATUS_FIELD],
    issues=[_status_now("6")],
    events=[_transition(101, SAME_INSTANT, "1", "3"), _transition(102, SAME_INSTANT, "3", "6")],
)
def test_a_chain_that_agrees_with_the_ids_keeps_their_order(scenario: Scenario) -> None:
    assert scenario.states(STATUS) == [["1"], ["3"], ["6"]]
    assert scenario.round_trip_holds()


@case(
    fields=[STATUS_FIELD],
    issues=[_status_now("1")],
    events=[_transition(101, SAME_INSTANT, "1", "3"), _transition(102, SAME_INSTANT, "3", "1")],
)
def test_a_cycle_within_one_instant_falls_back_to_the_changelog_id(scenario: Scenario) -> None:
    """A→B and B→A in one second form no unique chain: either could come first,
    so the changelog id decides, as it does for any event without a chain."""
    assert scenario.states(STATUS) == [["1"], ["3"], ["1"]]
    assert _changelog_seq(scenario) == {"101": 0, "102": 0}
    assert scenario.round_trip_holds()


@case(
    fields=[STATUS_FIELD],
    issues=[_status_now("6")],
    events=[
        _transition(104, SAME_INSTANT, "1", "3"),
        _transition(103, SAME_INSTANT, "3", "5"),
        _transition(102, SAME_INSTANT, "5", "7"),
        _transition(101, SAME_INSTANT, "7", "6"),
    ],
)
def test_a_four_step_chain_follows_its_links_however_the_ids_run(scenario: Scenario) -> None:
    """The ids count DOWN from the chain's head to its tail. The from→to chain
    still decides, walking a longer line than a single swap can exercise."""
    assert scenario.states(STATUS) == [["1"], ["3"], ["5"], ["7"], ["6"]]
    assert _changelog_seq(scenario) == {"104": 0, "103": 1, "102": 2, "101": 3}
    assert scenario.round_trip_holds()


@case(
    fields=[STATUS_FIELD],
    issues=[_status_now("5")],
    events=[_transition(201, SAME_INSTANT, "1", "3"), _transition(202, SAME_INSTANT, "1", "5")],
)
def test_a_fork_within_one_instant_falls_back_to_the_changelog_id(scenario: Scenario) -> None:
    """Two events leaving the SAME before form no unique chain either: nothing
    says which one happened first, so the changelog id decides."""
    assert scenario.states(STATUS) == [["1"], ["3"], ["5"]]
    assert _changelog_seq(scenario) == {"201": 0, "202": 0}
    assert scenario.round_trip_holds()


def test_one_changelog_id_naming_two_items_makes_its_group_ambiguous(scenario: Scenario) -> None:
    """A malformed entry can carry two items of the same field under one
    changelog id (as a duplicate entry can carry the same item twice). That id
    can no longer anchor a single position in any chain, so the fix is not to
    let `max()` hand it one anyway: the whole instant falls back to the
    changelog id, and the two items sharing id 100 collapse into the one
    journal row `unique_key` gives them — this scenario deliberately leaves the
    source's own events contradicting its current value, so it keeps its own
    warehouse rather than sharing the module's build."""
    scenario.seed(
        fields=[STATUS_FIELD],
        issues=[_status_now("6")],
        events=[
            event(
                "TST-1",
                100,
                SAME_INSTANT,
                [
                    item(STATUS, frm="1", frm_str=STATUS_NAMES["1"], to="3", to_str=STATUS_NAMES["3"]),
                    item(STATUS, frm="5", frm_str=STATUS_NAMES["5"], to="6", to_str=STATUS_NAMES["6"]),
                ],
            ),
            _transition(200, SAME_INSTANT, "3", "5"),
        ],
    )
    scenario.build()
    assert _changelog_seq(scenario) == {"100": 0, "200": 0}
    assert {r["event_id"] for r in scenario.journal(field=STATUS) if r["event_kind"] == "changelog"} == {"100", "200"}
