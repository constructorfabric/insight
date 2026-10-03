"""One changelog entry carrying several items of one self-describing field.

Every item of such a field states the whole value, so the entry is one event
(§5). Its items share the event's every sort key — instant, chain position,
changelog id — and the journal key, so leaving them as separate events let the
planner pick the surviving row, the value at creation and whether a
`snapshot_diff` follows. Each scenario builds twice from scratch and requires the
same journal both times, then the state the rule names.

None shares a build: the entries these scenarios seed are what
`assert_jira_entry_items_one_per_field` reports, so every neighbour's
invariants would read as failing.
"""

from __future__ import annotations

from typing import Any

from conftest import Scenario
from helpers import event, field, issue, item

LABELS = "customfield_10100"
LABELS_FIELD = field(
    LABELS,
    name="Example Options",
    schema_type="array",
    schema_items="option",
    schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:multiselect",
)
AT = "2026-01-06T10:00:00"

SET_X = item(LABELS, to="[10001]", to_str="X")
CLEAR_Y = item(LABELS, frm="[10002]", frm_str="Y")
SET_Z = item(LABELS, to="[10003]", to_str="Z")

DETECTOR = "assert_jira_entry_items_one_per_field"


def _options(*values: tuple[str, str]) -> list[dict[str, str]]:
    return [{"id": option_id, "value": display} for option_id, display in values]


def _journal_of_two_full_refreshes(scenario: Scenario) -> list[dict[str, Any]]:
    scenario.build()
    first = scenario.journal()
    scenario.build()
    second = scenario.journal()
    assert first, "the scenario produced no journal"
    assert second == first
    return second


def test_items_chained_through_the_empty_value_are_one_event_whatever_their_order(scenario: Scenario) -> None:
    """`Y -> ''` and `'' -> X` in one entry chain into `Y -> X`. Bronze lists
    them in either order, and neither order changes the event."""
    scenario.seed(
        fields=[LABELS_FIELD],
        issues=[
            issue("TST-1", fields={LABELS: _options(("10001", "X"))}),
            issue("TST-2", fields={LABELS: _options(("10001", "X"))}),
        ],
        events=[
            event("TST-1", 200, AT, [SET_X, CLEAR_Y]),
            event("TST-2", 300, AT, [CLEAR_Y, SET_X]),
        ],
    )
    _journal_of_two_full_refreshes(scenario)

    for key, entry in (("TST-1", "200"), ("TST-2", "300")):
        rows = scenario.journal(issue=key, field=LABELS)
        assert [(r["event_kind"], r["event_id"], r["value_ids"], r["value_displays"]) for r in rows] == [
            ("synthetic_initial", f"initial:{key}", ["10002"], ["Y"]),
            ("changelog", entry, ["10001"], ["X"]),
        ]
    assert scenario.round_trip_holds()
    assert not scenario.invariants_hold(DETECTOR)


def test_the_collapsed_event_links_into_its_instant_like_any_other(scenario: Scenario) -> None:
    """Entry 400 collapses to `Y -> X` and entry 399, at the same instant, goes
    `X -> W`. The ranks see the collapsed event too, so the chain puts 400
    first although its id is larger, and the field ends on the issue's value."""
    scenario.seed(
        fields=[LABELS_FIELD],
        issues=[issue("TST-1", fields={LABELS: _options(("10004", "W"))})],
        events=[
            event("TST-1", 400, AT, [SET_X, CLEAR_Y]),
            event("TST-1", 399, AT, [item(LABELS, frm="[10001]", frm_str="X", to="[10004]", to_str="W")]),
        ],
    )
    _journal_of_two_full_refreshes(scenario)

    rows = scenario.journal(field=LABELS)
    assert [(r["event_kind"], r["event_id"], r["value_ids"]) for r in rows] == [
        ("synthetic_initial", "initial:TST-1", ["10002"]),
        ("changelog", "400", ["10001"]),
        ("changelog", "399", ["10004"]),
    ]
    assert scenario.round_trip_holds()


def test_items_without_a_chain_keep_the_first_that_sets_a_value(scenario: Scenario) -> None:
    """Two items both leaving the empty value fork: neither follows the other.
    The entry keeps the first by content among those whose `to` holds a value,
    the issue's own value arrives as the observation it is, and the round trip
    keeps reporting the events that do not reach it."""
    scenario.seed(
        fields=[LABELS_FIELD],
        issues=[issue("TST-1", fields={LABELS: _options(("10001", "X"), ("10003", "Z"))})],
        events=[event("TST-1", 200, AT, [SET_Z, SET_X])],
    )
    _journal_of_two_full_refreshes(scenario)

    rows = scenario.journal(field=LABELS)
    assert [(r["event_kind"], r["value_ids"]) for r in rows] == [
        ("synthetic_initial", []),
        ("changelog", ["10001"]),
        ("snapshot_diff", ["10001", "10003"]),
    ]
    assert not scenario.round_trip_holds()


def test_an_entry_with_one_item_per_field_is_not_reported(scenario: Scenario) -> None:
    scenario.seed(
        fields=[LABELS_FIELD],
        issues=[issue("TST-1", fields={LABELS: _options(("10001", "X"))})],
        events=[event("TST-1", 200, AT, [item(LABELS, frm="[10002]", frm_str="Y", to="[10001]", to_str="X")])],
    )
    scenario.build()
    assert scenario.invariants_hold(DETECTOR)
