"""Synthetic rows: the state the changelog never states.

Bronze carries no initial value — only the current one and a list of changes.
Every `synthetic_initial` row is therefore derived, and these tests pin what it
is derived from and which fields get one at all. This is the half of the
contract that the replaced pipeline could not satisfy for any field outside its
hand-written list.
"""

from __future__ import annotations

import json

from helpers import CREATED_AT, field, issue

# One field per kind that reads differently, so a single scenario proves the
# whole dispatch rather than one branch of it.
ASSIGNEE = "assignee"
COMPONENTS = "components"
POINTS = "customfield_10001"
INCIDENT_AT = "customfield_10002"
SEVERITY = "customfield_10003"
LABELS = "labels"

CATALOGUE = [
    field(ASSIGNEE, name="Assignee", schema_type="user"),
    field(COMPONENTS, name="Components", schema_type="array", schema_items="component"),
    field(POINTS, name="Story Points", schema_type="number"),
    field(
        INCIDENT_AT,
        name="Incident Start",
        schema_type="datetime",
        schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:datetime",
    ),
    field(
        SEVERITY,
        name="Severity",
        schema_type="option",
        schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:select",
    ),
    field(LABELS, name="Labels", schema_type="array", schema_items="string"),
]

VALUES = {
    ASSIGNEE: {"accountId": "alice-acct", "displayName": "Alice Alpha"},
    COMPONENTS: [{"id": "501", "name": "api"}],
    POINTS: 5,
    INCIDENT_AT: "2026-02-01T07:00:00.000+0000",
    SEVERITY: {"id": "9001", "value": "High"},
    LABELS: ["alpha", "beta"],
}


def test_every_field_the_issue_carries_gets_an_initial_row(scenario):
    """Six fields of six kinds, not one changelog event between them.

    This is the shape of the reported defect: the issue holds all six values and
    the old pipeline modelled the two it happened to enumerate. Each kind also
    has to normalize correctly here, so one scenario covers the whole dispatch.
    """
    scenario.seed(fields=CATALOGUE, issues=[issue("TST-1", fields=VALUES)])
    scenario.build()

    rows = scenario.journal(issue="TST-1")
    assert [(r["field_id"], r["value_ids"]) for r in rows] == [
        ("assignee", ["alice-acct"]),
        ("components", ["501"]),
        ("created", []),
        ("customfield_10001", ["5"]),
        # canonicalized: the changelog spells the same instant without millis
        ("customfield_10002", ["2026-02-01 07:00:00.000"]),
        ("customfield_10003", ["9001"]),
        ("labels", ["alpha", "beta"]),
    ]
    assert {r["event_kind"] for r in rows} == {"synthetic_initial"}


def test_displays_come_from_the_field_shape_not_from_the_id(scenario):
    """The human-readable side is probed per element spelling: `value` for an
    option, `name` for a component, `displayName` for a user."""
    scenario.seed(fields=CATALOGUE, issues=[issue("TST-1", fields=VALUES)])
    scenario.build()

    displays = {r["field_id"]: r["value_displays"] for r in scenario.journal(issue="TST-1")}
    assert displays["assignee"] == ["Alice Alpha"]
    assert displays["components"] == ["api"]
    assert displays["customfield_10003"] == ["High"]
    assert displays["labels"] == ["alpha", "beta"]


def test_initial_rows_are_sequenced_by_field_id(scenario):
    """`_seq` orders the rows that share one timestamp.

    Every initial row of an issue is stamped with the same `created`, so
    `event_at` alone is not a total order. `_seq` is the field's 0-based index in
    `field_id`-ascending order, offset by one because the creation marker holds
    seq 0 — that is what makes `(event_at, _seq)` deterministic for a reader.
    """
    scenario.seed(fields=CATALOGUE, issues=[issue("TST-1", fields=VALUES)])
    scenario.build()

    assert [(r["field_id"], r["_seq"]) for r in scenario.journal(issue="TST-1")] == [
        ("assignee", 1),
        ("components", 2),
        ("created", 0),
        ("customfield_10001", 3),
        ("customfield_10002", 4),
        ("customfield_10003", 5),
        ("labels", 6),
    ]


def test_creation_marker_carries_the_reporter_and_no_value(scenario):
    """One row per issue that consumers read as "the issue exists from here".

    `task_issue_current_state.created_at` finds it by `event_kind`, so its shape
    is a contract: the sentinel field id, seq 0, the reporter as author, no
    value, and an event id under the `initial:` convention.
    """
    scenario.seed(fields=CATALOGUE, issues=[issue("TST-1", fields=VALUES, reporter_id="bob-acct")])
    scenario.build()

    marker = scenario.journal(issue="TST-1", field="created")
    assert len(marker) == 1
    assert marker[0]["event_kind"] == "synthetic_initial"
    assert marker[0]["_seq"] == 0
    assert marker[0]["author_id"] == "bob-acct"
    assert marker[0]["value_ids"] == []
    assert marker[0]["event_id"] == "initial:TST-1"
    assert marker[0]["event_at"].startswith(CREATED_AT.replace("T", " "))


def test_a_field_present_but_unset_gets_no_row(scenario):
    """ "Applicable and empty" is a real state, and it is deliberately not
    materialized: it is most of the key/value pairs an issue carries and stays
    recoverable from bronze. What must not happen is a row that claims a value.
    """
    scenario.seed(fields=CATALOGUE, issues=[issue("TST-1", fields={POINTS: None, LABELS: [], SEVERITY: None})])
    scenario.build()

    assert [r["field_id"] for r in scenario.journal(issue="TST-1")] == ["created"]


def test_container_fields_are_not_field_state(scenario):
    """Vote counts, watch counts and the time-tracking container are aggregates
    Jira computes, not values a person set. They are in the issue JSON, so
    without an explicit decision they would be normalized by whatever rule
    matched their structure."""
    scenario.seed(
        fields=[
            field("votes", name="Votes", schema_type="votes"),
            field("watches", name="Watchers", schema_type="watches"),
            field("timetracking", name="Time Tracking", schema_type="timetracking"),
            field(POINTS, name="Story Points", schema_type="number"),
        ],
        issues=[
            issue(
                "TST-1",
                fields={
                    "votes": {"votes": 3, "hasVoted": False},
                    "watches": {"watchCount": 2, "isWatching": True},
                    "timetracking": {"timeSpentSeconds": 300},
                    POINTS: 5,
                },
            )
        ],
    )
    scenario.build()

    assert [r["field_id"] for r in scenario.journal(issue="TST-1")] == ["created", POINTS]


def test_long_text_is_stored_by_content_address(scenario):
    """A description is kilobytes, and ClickHouse reads a whole column: a body
    inline in `value_displays` would be dragged through every read of every
    field. The journal keeps the hash and a prefix; the body goes to a side
    table, where identical bodies collapse into one row."""
    body = {
        "type": "doc",
        "version": 1,
        "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Steps to reproduce"}]}],
    }
    scenario.seed(
        fields=[field("description", name="Description", schema_type="string")],
        issues=[issue("TST-1", fields={"description": body})],
    )
    scenario.build()

    rows = scenario.journal(issue="TST-1", field="description")
    assert len(rows) == 1
    text_id = rows[0]["value_ids"][0]
    assert len(text_id) == 32, f"expected a 128-bit hex address, got {text_id!r}"
    assert "Steps to reproduce" in rows[0]["value_displays"][0]

    side = scenario.text_rows()
    assert [r["text_id"] for r in side] == [text_id]
    assert side[0]["content_form"] == "adf_json"
    assert json.loads(side[0]["content"]) == body


def test_a_long_text_prefix_never_splits_a_character(scenario):
    """The prefix is measured in characters, not bytes.

    ClickHouse's `substring` counts bytes, so a body whose 200th character
    boundary falls mid-character comes back as invalid UTF-8 — stored happily,
    accepted by every array-shape test, and rejected by the first consumer that
    decodes strictly.
    """
    # Cyrillic: two bytes per character, so a byte-based cut lands mid-character.
    body = {
        "type": "doc",
        "version": 1,
        "content": [{"type": "paragraph", "content": [{"type": "text", "text": "привет " * 60}]}],
    }
    scenario.seed(
        fields=[field("description", name="Description", schema_type="string")],
        issues=[issue("TST-1", fields={"description": body})],
    )
    scenario.build()

    prefix = scenario.journal(issue="TST-1", field="description")[0]["value_displays"][0]
    assert prefix == prefix.encode("utf-8").decode("utf-8")
    assert scenario.invariants_hold("assert_jira_values_are_valid_utf8")
