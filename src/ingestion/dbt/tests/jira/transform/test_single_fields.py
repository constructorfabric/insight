"""Single-valued fields: one value at a time, every event self-describing.

These are the cases the replaced pipeline got wrong by omission — a field it did
not enumerate produced no row at all, however much history it had. Each test
states the whole journal for the issue, so a row appearing that should not is a
failure just as much as a row missing.
"""

from __future__ import annotations

from conftest import Scenario
from helpers import CREATED_AT, event, field, issue, item

STORY_POINTS = "customfield_10001"
CREATED_MARKER = ("created", "synthetic_initial")


def _shape(rows):
    """(field_id, event_kind, value_ids) — what a scenario is usually about."""
    return [(r["field_id"], r["event_kind"], r["value_ids"]) for r in rows]


def test_field_set_at_creation_and_never_changed_still_has_history(scenario: Scenario) -> None:
    """The defect this whole change exists for.

    A custom field the issue carries, with no changelog event of its own, must
    still reach the journal. The replaced pipeline seeded state from a
    hand-written list of about ten system fields and walked the changelog
    backwards, so a field outside that list with no event produced nothing.
    """
    scenario.seed(
        fields=[field(STORY_POINTS, name="Story Points", schema_type="number")],
        issues=[issue("TST-1", fields={STORY_POINTS: 5})],
    )
    scenario.build()

    assert _shape(scenario.journal(issue="TST-1")) == [
        (*CREATED_MARKER, []),
        (STORY_POINTS, "synthetic_initial", ["5"]),
    ]


def test_value_at_creation_is_recovered_from_the_first_event(scenario: Scenario) -> None:
    """Bronze holds no initial state — it is derived, and this is the rule.

    The issue currently holds 5 and changed twice. The value it was created with
    is the `from` side of its earliest event, which is the one thing that says
    so; every later state is the `to` side of its own event.
    """
    scenario.seed(
        fields=[field(STORY_POINTS, name="Story Points", schema_type="number")],
        issues=[issue("TST-1", fields={STORY_POINTS: 5})],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [item(STORY_POINTS, frm="3", frm_str="3", to="4", to_str="4")]),
            event("TST-1", 102, "2026-01-07T10:00:00", [item(STORY_POINTS, frm="4", frm_str="4", to="5", to_str="5")]),
        ],
    )
    scenario.build()

    assert scenario.states(STORY_POINTS) == [["3"], ["4"], ["5"]]
    assert [r["event_kind"] for r in scenario.journal(field=STORY_POINTS)] == [
        "synthetic_initial",
        "changelog",
        "changelog",
    ]


def test_initial_row_is_dated_by_the_issue_creation(scenario: Scenario) -> None:
    """A synthetic row is stamped from the issue's own `created`, not from the
    event it was derived from — otherwise the field looks as though it appeared
    when it first changed."""
    scenario.seed(
        fields=[field(STORY_POINTS, name="Story Points", schema_type="number")],
        issues=[issue("TST-1", fields={STORY_POINTS: 5})],
        events=[
            event("TST-1", 101, "2026-02-20T10:00:00", [item(STORY_POINTS, frm="3", frm_str="3", to="5", to_str="5")])
        ],
    )
    scenario.build()

    initial = [r for r in scenario.journal(field=STORY_POINTS) if r["event_kind"] == "synthetic_initial"]
    assert len(initial) == 1
    assert initial[0]["event_at"].startswith(CREATED_AT.replace("T", " "))


def test_field_cleared_ends_empty_and_agrees_with_the_issue(scenario: Scenario) -> None:
    """Clearing a field is an ordinary event, not a degenerate one: the `from`
    side still names what was removed, and the resulting state is empty."""
    scenario.seed(
        fields=[field(STORY_POINTS, name="Story Points", schema_type="number")],
        issues=[issue("TST-1", fields={STORY_POINTS: None})],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [item(STORY_POINTS, frm="3", frm_str="3", to=None, to_str=None)])
        ],
    )
    scenario.build()

    assert scenario.states(STORY_POINTS) == [["3"], []]


def test_field_absent_from_the_issue_context_produces_no_row(scenario: Scenario) -> None:
    """A key the issue JSON does not carry means the field is not in this
    issue's field configuration. That is not an empty value — there is nothing
    to measure, so there is no row."""
    scenario.seed(
        fields=[field(STORY_POINTS, name="Story Points", schema_type="number")], issues=[issue("TST-1", fields={})]
    )
    scenario.build()

    assert _shape(scenario.journal(issue="TST-1")) == [(*CREATED_MARKER, [])]


# ── the time-tracking estimates: zero is not a value ────────────────────────

REMAINING = "timeestimate"
REMAINING_FIELD = field(REMAINING, name="Remaining Estimate", schema_type="number")


def test_work_logged_on_an_unestimated_issue_leaves_no_estimate(scenario: Scenario) -> None:
    """Jira emits `timeestimate: null -> 0` when work is logged against an issue
    that was never estimated, and the issue resource still reports `null`.

    Read literally the journal would end at "0" and disagree with the issue on
    every such row. It is the same state under two spellings, and neither side
    can express a deliberate zero as distinct from it (§3.5).
    """
    scenario.seed(
        fields=[REMAINING_FIELD],
        issues=[issue("TST-1", fields={REMAINING: None})],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [item(REMAINING, frm=None, frm_str=None, to="0", to_str="0")])
        ],
    )
    scenario.build()

    assert scenario.states(REMAINING) == [[], []]
    assert scenario.round_trip_holds()


def test_an_estimate_consumed_to_zero_ends_empty(scenario: Scenario) -> None:
    """A real estimate worked down to nothing: the resource reports `null` here
    too, so the journal must land on the same empty state."""
    scenario.seed(
        fields=[REMAINING_FIELD],
        issues=[issue("TST-1", fields={REMAINING: None})],
        events=[
            event(
                "TST-1", 101, "2026-01-06T10:00:00", [item(REMAINING, frm="28800", frm_str="28800", to="0", to_str="0")]
            )
        ],
    )
    scenario.build()

    assert scenario.states(REMAINING) == [["28800"], []]
    assert scenario.round_trip_holds()


def test_a_story_point_estimate_of_zero_is_still_a_value(scenario: Scenario) -> None:
    """The zero-is-nothing rule is named for the time-tracking fields, not
    inferred from a value being numeric. A story-point field is the same
    structure, and a zero there is something somebody typed."""
    scenario.seed(
        fields=[field(STORY_POINTS, name="Story Points", schema_type="number")],
        issues=[issue("TST-1", fields={STORY_POINTS: 0})],
    )
    scenario.build()

    assert scenario.states(STORY_POINTS) == [["0"]]
    assert scenario.round_trip_holds()
