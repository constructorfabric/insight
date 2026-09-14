"""Order, duplicates and repeated syncs.

Bronze is append-only and Airbyte re-emits, so the same changelog arrives many
times and the same issue arrives in several versions. None of that may change
the journal: `unique_key` is a pure function of content, so two runs over the
same bronze produce byte-identical keys and ReplacingMergeTree collapses them.

Order matters for one kind only — the element-wise one, where an inverted pair
of events changes the resulting set — so that is where it is tested.
"""

from __future__ import annotations

from conftest import Scenario
from helpers import CREATED_AT, LATER_SYNC, event, field, issue, item

COMPONENTS = "components"
POINTS = "customfield_10001"
COMPONENTS_FIELD = field(COMPONENTS, name="Components", schema_type="array", schema_items="component")
POINTS_FIELD = field(POINTS, name="Story Points", schema_type="number")

SAME_INSTANT = "2026-01-06T10:00:00"


def test_events_sharing_an_instant_are_ordered_by_changelog_id(scenario: Scenario) -> None:
    """Two events in the same second, one adding an element and one removing it.

    Their relative order decides whether the field ends up holding the element,
    so the tie-break has to follow Jira's own monotonic changelog id — and
    numerically, not as text: ids cross a digit-count boundary all the time, and
    `'101' < '99'` as strings.
    """
    scenario.seed(
        fields=[COMPONENTS_FIELD],
        issues=[issue("TST-1", fields={COMPONENTS: []})],
        events=[
            event("TST-1", 99, SAME_INSTANT, [item(COMPONENTS, to="501", to_str="api")]),
            event("TST-1", 101, SAME_INSTANT, [item(COMPONENTS, frm="501", frm_str="api")]),
        ],
    )
    scenario.build()

    assert scenario.states(COMPONENTS) == [[], ["501"], []]
    assert scenario.round_trip_holds()


def test_the_same_changelog_re_emitted_produces_one_row(scenario: Scenario) -> None:
    """Airbyte appends, so a changelog the connector has seen before arrives
    again on the next sync. Two rows in bronze, one event in the journal."""
    original = event("TST-1", 101, SAME_INSTANT, [item(POINTS, frm="3", frm_str="3", to="5", to_str="5")])
    resync = dict(original, _airbyte_extracted_at=LATER_SYNC)
    scenario.seed(fields=[POINTS_FIELD], issues=[issue("TST-1", fields={POINTS: 5})], events=[original, resync])
    scenario.build()

    assert scenario.states(POINTS) == [["3"], ["5"]]
    assert scenario.round_trip_holds()


def test_an_item_repeated_inside_one_changelog_produces_one_row(scenario: Scenario) -> None:
    """Jira sometimes puts the same (field, from, to) twice in one entry's
    items array. Both would carry the same event id, so the second is not a
    second event."""
    duplicated = item(POINTS, frm="3", frm_str="3", to="5", to_str="5")
    scenario.seed(
        fields=[POINTS_FIELD],
        issues=[issue("TST-1", fields={POINTS: 5})],
        events=[event("TST-1", 101, SAME_INSTANT, [duplicated, duplicated])],
    )
    scenario.build()

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


def test_a_newer_issue_version_supersedes_the_older(scenario: Scenario) -> None:
    """The issue's current value comes from ONE chosen bronze row. Resolving it
    per column instead lets two syncs mix, which is how an issue ends up with a
    status from one version and a value from another."""
    scenario.seed(
        fields=[POINTS_FIELD],
        issues=[issue("TST-1", fields={POINTS: 3}), issue("TST-1", fields={POINTS: 8}, extracted_at=LATER_SYNC)],
    )
    scenario.build()

    assert scenario.states(POINTS) == [["8"]]
    assert scenario.round_trip_holds()


def test_an_issue_moved_between_projects_records_the_move(scenario: Scenario) -> None:
    """`project` is an ordinary object field with real changelog traffic, not a
    container: an issue moved between projects has to show it."""
    scenario.seed(
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
    scenario.build()

    rows = scenario.journal(field="project")
    assert [r["value_ids"] for r in rows] == [["901"], ["902"]]
    # `name` must win over `key` in the display probe, or the two sides of the
    # pipeline stop agreeing on what a project object is called.
    assert rows[-1]["value_displays"] == ["New Project"]
    assert scenario.round_trip_holds()


def test_an_event_on_the_creation_instant_still_wins(scenario: Scenario) -> None:
    """An issue whose first event happened at its own creation.

    Both rows then carry the same `event_at`, and `_seq` sorts them the wrong
    way round: it is 0 for the changelog row and 1..N for the initial one. A
    reader ordering by `(event_at, _seq)` — which the contract calls a total
    order — reads the field as still empty. The kind is what breaks the tie,
    because an initial row is by definition the state before any event.
    """
    scenario.seed(
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
    scenario.build()

    assert scenario.states("customfield_11300") == [[], ["TST-9"]]
    assert scenario.round_trip_holds()
