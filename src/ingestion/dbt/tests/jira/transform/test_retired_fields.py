"""Withdrawal of a field from an issue (spec §3.6).

Jira emits no changelog item when a field leaves a project's or an issue type's
configuration, or when the field is deleted outright: the key simply stops
appearing in the issue JSON. Without an event the journal's newest state stays
at a value the issue no longer has.

The tests here pin both halves of the rule — that an absent key produces the
withdrawal, and that a key present-and-empty does NOT. The second is the more
important one: a synthetic row emitted there would paper over a genuinely lost
clearing event, which is the defect class this whole change exists to expose.
"""

from __future__ import annotations

from typing import Any

from conftest import Scenario
from helpers import OBSERVED_AT, event, field, issue, item

SEVERITY = "customfield_10003"
PRODUCTS = "customfield_10004"

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

LATER = "2026-04-01T12:00:00"


def _severity_history(key: str = "TST-1") -> list[dict[str, Any]]:
    return [event(key, 101, "2026-01-06T10:00:00", [item(SEVERITY, frm=None, frm_str=None, to="9001", to_str="High")])]


def test_absent_key_after_history_yields_one_withdrawal(scenario: Scenario) -> None:
    """The field was set, then left the issue's configuration.

    The journal must end at "holds nothing", and it must say so as an event
    rather than by having its last row quietly disagree with the issue.
    """
    scenario.seed(fields=[SEVERITY_FIELD], issues=[issue("TST-1", fields={})], events=_severity_history())
    scenario.build()

    rows = scenario.journal(field=SEVERITY)
    assert [(r["event_kind"], r["value_ids"]) for r in rows] == [
        ("synthetic_initial", []),
        ("changelog", ["9001"]),
        ("retired_field", []),
    ]
    # The withdrawal is what makes the pair reconcile: without it the newest
    # state is a value the issue does not have.
    assert scenario.round_trip_holds()


def test_the_withdrawal_is_dated_by_the_observation(scenario: Scenario) -> None:
    """Jira exposes no date for a configuration change, so the honest stamp is
    when the absence was seen — the issue's own extraction mark. That is also
    the stamp the round trip treats as the issue's freshness, so the event can
    never be newer than the state it is compared against."""
    scenario.seed(fields=[SEVERITY_FIELD], issues=[issue("TST-1", fields={})], events=_severity_history())
    scenario.build()

    withdrawal = [r for r in scenario.journal(field=SEVERITY) if r["event_kind"] == "retired_field"]
    assert len(withdrawal) == 1
    assert withdrawal[0]["event_id"] == "retired:TST-1"
    assert withdrawal[0]["event_at"].startswith(OBSERVED_AT.replace("T", " "))
    # No actor: withdrawing a field is a configuration change, not an edit.
    assert withdrawal[0]["author_id"] is None


def test_a_present_but_empty_key_is_not_a_withdrawal(scenario: Scenario) -> None:
    """The field still applies to the issue and is unset — an ordinary state.

    If the journal disagrees with it, a clearing event is missing and that must
    surface as a round-trip failure. Absorbing it into a synthetic row would
    blind the only oracle that catches a mis-parsed event.
    """
    scenario.seed(fields=[SEVERITY_FIELD], issues=[issue("TST-1", fields={SEVERITY: None})], events=_severity_history())
    scenario.build()

    kinds = [r["event_kind"] for r in scenario.journal(field=SEVERITY)]
    assert "retired_field" not in kinds
    assert kinds == ["synthetic_initial", "changelog"]


def test_a_withdrawal_keeps_the_field_identifier_type(scenario: Scenario) -> None:
    """`value_id_type` is asserted stable per (source, field), so a row of that
    field may not carry a different one just because its arrays are empty."""
    scenario.seed(fields=[SEVERITY_FIELD], issues=[issue("TST-1", fields={})], events=_severity_history())
    scenario.build()

    types = {r["value_id_type"] for r in scenario.journal(field=SEVERITY)}
    assert types == {"opaque_id"}


def test_cardinality_decides_how_the_withdrawal_reads(scenario: Scenario) -> None:
    """A single field is `set` to nothing; a multi field has its elements
    removed. Same rule the cardinality contract states for a value going away.
    """
    scenario.seed(
        fields=[SEVERITY_FIELD, PRODUCTS_FIELD],
        issues=[issue("TST-1", fields={})],
        events=_severity_history()
        + [
            event(
                "TST-1",
                102,
                "2026-01-07T10:00:00",
                [item(PRODUCTS, frm=None, frm_str=None, to="[7001]", to_str="Storage")],
            )
        ],
    )
    scenario.build()

    withdrawals = {
        r["field_id"]: (r["field_cardinality"], r["delta_action"])
        for r in scenario.journal(issue="TST-1")
        if r["event_kind"] == "retired_field"
    }
    assert withdrawals == {SEVERITY: ("single", "set"), PRODUCTS: ("multi", "remove")}


def test_a_field_that_comes_back_leaves_no_withdrawal_behind(scenario: Scenario) -> None:
    """A configuration change is reversible, so the withdrawal must not be.

    The second sync is modelled the way Airbyte produces one — another row for
    the same issue with a later extraction mark, not an edit — so this also
    exercises the dedup that picks the newer of the two.
    """
    scenario.seed(fields=[SEVERITY_FIELD], issues=[issue("TST-1", fields={})], events=_severity_history())
    scenario.build()
    assert [r["event_kind"] for r in scenario.journal(field=SEVERITY)][-1] == "retired_field"

    scenario.seed(
        fields=[],
        issues=[issue("TST-1", fields={SEVERITY: {"id": "9001", "value": "High"}}, extracted_at=LATER)],
        events=[],
    )
    scenario.build()

    assert [(r["event_kind"], r["value_ids"]) for r in scenario.journal(field=SEVERITY)] == [
        ("synthetic_initial", []),
        ("changelog", ["9001"]),
    ]
