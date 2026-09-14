"""An issue moved between projects is one issue, under one key.

Jira renames an issue when it moves (`OLD-1` becomes `NEW-9`) and keeps its
numeric id. Bronze then holds the issue twice — one row per key the connector
ever saw — and its changelog entries under whichever key was current when each
was fetched. Everything here has to fold that back into one history: one
creation marker, one initial state, every event, all keyed by the id.
"""

from __future__ import annotations

from conftest import Scenario
from helpers import CREATED_AT, OBSERVED_AT, SOURCE_ID, event, field, issue, item

STATUS = field("status", name="Status", schema_type="status")
EARLIER = "2026-02-01T12:00:00"


def test_a_moved_issue_has_one_history_under_its_current_key(scenario: Scenario) -> None:
    scenario.seed(
        fields=[STATUS],
        issues=[
            # The row written before the move, under the old key and never replaced.
            issue("OLD-1", jira_id="7001", fields={"status": {"id": "3", "name": "In Progress"}}, extracted_at=EARLIER),
            issue("NEW-9", jira_id="7001", fields={"status": {"id": "6", "name": "Closed"}}, extracted_at=OBSERVED_AT),
        ],
        events=[
            # Fetched before the move: stamped with the old key, backfilled with the id.
            event(
                "OLD-1",
                101,
                "2026-01-06T10:00:00",
                [item("status", frm="1", frm_str="Open", to="3", to_str="In Progress")],
                jira_id="7001",
            ),
            event(
                "NEW-9",
                102,
                "2026-01-20T10:00:00",
                [item("status", frm="3", frm_str="In Progress", to="6", to_str="Closed")],
                jira_id="7001",
            ),
        ],
    )
    scenario.build()

    assert scenario.journal(issue="OLD-1") == [], "the old key must not survive as a second issue"

    rows = scenario.journal(issue="NEW-9")
    assert [(r["field_id"], r["event_kind"], r["event_id"], r["value_ids"]) for r in rows] == [
        ("created", "synthetic_initial", "initial:7001", []),
        ("status", "synthetic_initial", "initial:7001", ["1"]),
        ("status", "changelog", "101", ["3"]),
        ("status", "changelog", "102", ["6"]),
    ]
    assert {r["event_at"] for r in rows if r["event_kind"] == "synthetic_initial"} == {
        CREATED_AT.replace("T", " ") + ".000"
    }

    keys = scenario.warehouse.rows(
        "SELECT unique_key FROM staging.jira__field_history_derived FINAL WHERE insight_source_id = {src:String}",
        {"src": SOURCE_ID},
    )
    assert all(k["unique_key"].startswith(f"{SOURCE_ID}-jira-7001-") for k in keys), keys
    assert scenario.invariants_hold()


def test_an_event_without_an_issue_id_is_left_out_and_counted(scenario: Scenario) -> None:
    """A changelog entry can name an issue no issue row exists for — deleted
    between two syncs, or never fetched. It carries no id, so it is not an event
    of any issue the journal knows; it is left out, and the connector check
    reports it rather than letting the omission pass silently."""
    scenario.seed(
        fields=[STATUS],
        issues=[],
        events=[
            event(
                "GONE-3",
                201,
                "2026-01-06T10:00:00",
                [item("status", frm="1", frm_str="Open", to="3", to_str="In Progress")],
                jira_id=None,
            )
        ],
    )
    scenario.build()

    assert scenario.journal(issue="GONE-3") == []
    assert scenario.warehouse.rows(
        "SELECT count() AS n FROM staging.jira__field_history_derived FINAL WHERE insight_source_id = {src:String} AND field_id = 'status'",
        {"src": SOURCE_ID},
    ) == [{"n": 0}]
    # The omission is a finding of the connector check, not a silence.
    assert not scenario.invariants_hold("assert_jira_substream_rows_without_issue_id")
    assert scenario.invariants_hold("assert_jira_field_history_key_is_issue_keyed")
