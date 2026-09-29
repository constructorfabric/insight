"""A value the issue holds that its events never reach (spec §6.1).

Jira can change a value without writing a changelog entry, and history imported
from another tracker can be missing entries outright. The journal then ends on a
value the issue does not hold, and every consumer that reads the newest state
serves the stale one. A `snapshot_diff` row records the value observed.

The date is the hard part: the row is re-derived every time the issue is, so a
date taken from the sync would move a closure forward on each run.

None of these scenarios asserts the round trip: `snapshot_diff` is excluded
from it by design, so a pair that needed one keeps failing it.
"""

from __future__ import annotations

from typing import Any

from conftest import Scenario, case
from helpers import LATER_SYNC, event, field, issue, item, status

SEVERITY = "customfield_10003"
PRODUCTS = "customfield_10004"

STATUS_FIELD = field("status", name="Status", schema_type="status")
SEVERITY_FIELD = field(
    SEVERITY,
    name="Severity",
    schema_type="option",
    schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:select",
)
PRODUCTS_FIELD = field(
    PRODUCTS,
    name="Products",
    schema_type="array",
    schema_items="option",
    schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:multiselect",
)

STATUSES = [
    status("1", name="Open", category_key="new"),
    status("3", name="In Progress", category_key="indeterminate"),
    status("6", name="Closed", category_key="done"),
]

LAST_EVENT_AT = "2026-01-06T10:00:00"
ONE_MS_AFTER_LAST_EVENT = "2026-01-06 10:00:00.001"
RESOLVED_AT = "2026-02-10T15:30:00.000+0000"
RESOLVED_BEFORE_LAST_EVENT = "2026-01-05T12:00:00.000+0000"


def _started() -> list[dict[str, Any]]:
    return [event("TST-1", 101, LAST_EVENT_AT, [item("status", frm="1", frm_str="Open", to="3", to_str="In Progress")])]


def _severity_set_high() -> list[dict[str, Any]]:
    return [event("TST-1", 101, LAST_EVENT_AT, [item(SEVERITY, frm=None, frm_str=None, to="9001", to_str="High")])]


def _diff_rows(scenario: Scenario, field_id: str) -> list[dict[str, Any]]:
    return [r for r in scenario.journal(field=field_id) if r["event_kind"] == "snapshot_diff"]


@case(
    fields=[STATUS_FIELD],
    issues=[issue("TST-1", fields={"status": {"id": "6", "name": "Closed"}, "resolutiondate": RESOLVED_AT})],
    events=_started(),
    statuses=STATUSES,
)
def test_an_unrecorded_closure_is_dated_by_the_resolution(scenario: Scenario) -> None:
    """The issue is done in Jira and its history stops at In Progress.

    The resolution is the one stable date Jira keeps for that closure, and it is
    after the last recorded event, so it is where the missing transition goes.
    """
    rows = scenario.journal(field="status")
    assert [(r["event_kind"], r["value_ids"]) for r in rows] == [
        ("synthetic_initial", ["1"]),
        ("changelog", ["3"]),
        ("snapshot_diff", ["6"]),
    ]
    diff = rows[-1]
    assert diff["value_displays"] == ["Closed"]
    assert diff["event_at"] == "2026-02-10 15:30:00.000"
    assert (diff["event_id"], diff["delta_action"], diff["author_id"]) == ("snapshot_diff:TST-1", "set", None)


@case(
    fields=[STATUS_FIELD],
    issues=[issue("TST-1", fields={"status": {"id": "6", "name": "Closed"}})],
    events=_started(),
    statuses=STATUSES,
)
def test_a_closure_without_a_resolution_sorts_just_after_the_last_event(scenario: Scenario) -> None:
    """With no resolution to date it, the earliest moment the change can have
    happened is right after the last event the history does record."""
    assert [r["event_at"] for r in _diff_rows(scenario, "status")] == [ONE_MS_AFTER_LAST_EVENT]


@case(
    fields=[STATUS_FIELD],
    issues=[issue("TST-1", fields={"status": {"id": "1", "name": "Open"}, "resolutiondate": RESOLVED_AT})],
    events=_started(),
    statuses=STATUSES,
)
def test_a_resolution_dates_only_a_status_that_is_done(scenario: Scenario) -> None:
    """A resolution date left on a reopened issue says nothing about when it
    became Open again."""
    rows = _diff_rows(scenario, "status")
    assert [(r["value_ids"], r["event_at"]) for r in rows] == [(["1"], ONE_MS_AFTER_LAST_EVENT)]


@case(
    fields=[STATUS_FIELD],
    issues=[
        issue("TST-1", fields={"status": {"id": "6", "name": "Closed"}, "resolutiondate": RESOLVED_BEFORE_LAST_EVENT})
    ],
    events=_started(),
    statuses=STATUSES,
)
def test_a_resolution_before_the_last_event_does_not_date_the_closure(scenario: Scenario) -> None:
    """A resolution earlier than the last recorded event cannot date the missing
    closure: the earliest moment it can have happened is right after that event."""
    rows = _diff_rows(scenario, "status")
    assert [(r["value_ids"], r["event_at"]) for r in rows] == [(["6"], ONE_MS_AFTER_LAST_EVENT)]


@case(
    fields=[SEVERITY_FIELD],
    issues=[issue("TST-1", fields={SEVERITY: {"id": "9002", "value": "Low"}})],
    events=_severity_set_high(),
)
def test_any_field_that_changed_unrecorded_gets_the_observed_value(scenario: Scenario) -> None:
    rows = _diff_rows(scenario, SEVERITY)
    assert [(r["value_ids"], r["value_displays"], r["delta_action"], r["event_at"]) for r in rows] == [
        (["9002"], ["Low"], "set", ONE_MS_AFTER_LAST_EVENT)
    ]


@case(
    fields=[SEVERITY_FIELD],
    issues=[issue("TST-1", fields={SEVERITY: {"id": "9101", "value": "High"}})],
    events=_severity_set_high(),
)
def test_a_value_recreated_under_a_new_id_is_not_a_change(scenario: Scenario) -> None:
    """A migrated instance recreates option values under new ids (§3.4). The
    issue still holds High; writing it back as a change would flood the journal
    with events that never happened."""
    assert _diff_rows(scenario, SEVERITY) == []


@case(
    fields=[SEVERITY_FIELD],
    issues=[issue("TST-1", fields={SEVERITY: {"id": "9001", "value": "Critical"}})],
    events=_severity_set_high(),
)
def test_a_renamed_value_is_not_a_change(scenario: Scenario) -> None:
    """Same id, new label: the option was renamed, the issue's value is the same."""
    assert _diff_rows(scenario, SEVERITY) == []


@case(
    fields=[SEVERITY_FIELD],
    issues=[issue("TST-1", fields={SEVERITY: {"id": "9002", "value": "Low"}})],
    events=[
        event("TST-1", 101, "2026-03-05T10:00:00", [item(SEVERITY, frm=None, frm_str=None, to="9001", to_str="High")])
    ],
)
def test_events_newer_than_the_issue_row_are_not_contradicted(scenario: Scenario) -> None:
    """The changelog substream is read after the issue page, so it can hold an
    event the issue row predates. The snapshot is the stale side then."""
    assert _diff_rows(scenario, SEVERITY) == []


@case(
    fields=[PRODUCTS_FIELD],
    issues=[
        issue("TST-1", fields={PRODUCTS: [{"id": "7001", "value": "Storage"}, {"id": "7002", "value": "Network"}]})
    ],
    events=[
        event("TST-1", 101, LAST_EVENT_AT, [item(PRODUCTS, frm=None, frm_str=None, to="[7001]", to_str="Storage")])
    ],
)
def test_a_multi_value_field_is_replaced_by_the_observed_set(scenario: Scenario) -> None:
    rows = _diff_rows(scenario, PRODUCTS)
    assert [(r["value_ids"], r["value_displays"], r["delta_action"]) for r in rows] == [
        (["7001", "7002"], ["Storage", "Network"], "set")
    ]


DESCRIPTION_ADF = {
    "type": "doc",
    "version": 1,
    "content": [{"type": "paragraph", "content": [{"type": "text", "text": "New body"}]}],
}


@case(
    fields=[field("description", name="Description", schema_type="string")],
    issues=[issue("TST-1", fields={"description": DESCRIPTION_ADF})],
    events=[
        event(
            "TST-1", 101, LAST_EVENT_AT, [item("description", frm=None, frm_str="Old body", to=None, to_str="New body")]
        )
    ],
)
def test_long_text_is_never_contradicted_by_its_snapshot(scenario: Scenario) -> None:
    """The issue JSON holds an ADF document and the changelog holds rendered
    text, so their content addresses never agree and `long_text` never gets a
    `differs` row."""
    assert _diff_rows(scenario, "description") == []


def _seed_unrecorded_closure(scenario: Scenario) -> None:
    scenario.seed(
        fields=[STATUS_FIELD],
        issues=[issue("TST-1", fields={"status": {"id": "6", "name": "Closed"}})],
        events=_started(),
        statuses=STATUSES,
    )
    scenario.build()


def test_the_date_does_not_move_when_the_issue_is_synced_again(scenario: Scenario) -> None:
    """A later sync recomputes the issue. A date taken from that sync would move
    the closure forward on every run, into whichever period the run falls in."""
    _seed_unrecorded_closure(scenario)
    before = [r["event_at"] for r in _diff_rows(scenario, "status")]

    scenario.seed(
        fields=[],
        issues=[issue("TST-1", fields={"status": {"id": "6", "name": "Closed"}}, extracted_at=LATER_SYNC)],
        events=[],
    )
    scenario.build(full_refresh=False)

    assert before == [ONE_MS_AFTER_LAST_EVENT]
    assert [r["event_at"] for r in _diff_rows(scenario, "status")] == before


def test_the_row_goes_away_once_the_event_arrives(scenario: Scenario) -> None:
    """The observed value stands in for an event only until the event exists."""
    _seed_unrecorded_closure(scenario)

    scenario.seed(
        fields=[],
        issues=[issue("TST-1", fields={"status": {"id": "6", "name": "Closed"}}, extracted_at=LATER_SYNC)],
        events=[
            event(
                "TST-1",
                102,
                "2026-02-11T09:00:00",
                [item("status", frm="3", frm_str="In Progress", to="6", to_str="Closed")],
                extracted_at=LATER_SYNC,
            )
        ],
    )
    scenario.build(full_refresh=False)

    assert [(r["event_kind"], r["value_ids"]) for r in scenario.journal(field="status")] == [
        ("synthetic_initial", ["1"]),
        ("changelog", ["3"]),
        ("changelog", ["6"]),
    ]
