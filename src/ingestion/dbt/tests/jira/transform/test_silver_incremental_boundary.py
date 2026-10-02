"""The class table admits a producer's rows whatever another producer stamped.

`silver.class_task_field_history` is written by several producers, each
stamping `_version` on its own clock: the Jira journal by bronze extraction
time, the availability and lifecycle arms by build time, other trackers by
their own rules. An incremental run admits rows above a boundary, and a
boundary taken over the whole table lets one producer's newest row hide every
later row of another producer whose clock runs behind — for good, since nothing
reads those rows again.

Each test plants a row from another producer far in the future, then lands a
new issue through the journal and builds the class incrementally. The new
issue's rows must arrive.
"""

from __future__ import annotations

from conftest import Scenario
from helpers import LATER_SYNC, event, field, issue, item, status

FAR_FUTURE_MS = 4_102_444_800_000

STATUS_FIELD = field("status", name="Status", schema_type="status")
STATUSES = [
    status("1", name="Open", category_key="new"),
    status("3", name="In Progress", category_key="indeterminate"),
]


def _started(key: str, *, extracted_at: str | None = None) -> dict:
    extra = {"extracted_at": extracted_at} if extracted_at else {}
    return event(
        key, 101, "2026-01-06T10:00:00", [item("status", frm="1", frm_str="Open", to="3", to_str="In Progress")], **extra
    )


def _issue(key: str, *, extracted_at: str | None = None) -> dict:
    extra = {"extracted_at": extracted_at} if extracted_at else {}
    return issue(key, fields={"status": {"id": "3", "name": "In Progress"}}, **extra)


def _build_class(scenario: Scenario) -> None:
    # Incremental on purpose: the boundary under test only exists on an
    # incremental run, and a full refresh would drop rows other modules seed.
    scenario.warehouse.dbt("run", "--select", "class_task_field_history")


def _issues_in_class(scenario: Scenario) -> set[str]:
    rows = scenario.warehouse.rows(
        "SELECT DISTINCT issue_id FROM silver.class_task_field_history FINAL"
        " WHERE insight_source_id = {src:String} AND data_source = 'jira'"
        " AND event_kind IN ('synthetic_initial', 'changelog')",
        {"src": scenario.source},
    )
    return {r["issue_id"] for r in rows}


def _plant_future_row(scenario: Scenario, *, source: str, data_source: str, event_kind: str, field_id: str) -> None:
    """A row another producer wrote, versioned far ahead of the journal's clock."""
    scenario.warehouse.execute(
        "INSERT INTO silver.class_task_field_history SELECT * REPLACE ("
        "  concat(unique_key, '-planted') AS unique_key,"
        "  {source:String} AS insight_source_id,"
        "  {data_source:String} AS data_source,"
        "  {event_kind:String} AS event_kind,"
        "  {field_id:String} AS field_id,"
        "  {version:UInt64} AS _version"
        ") FROM silver.class_task_field_history WHERE insight_source_id = {src:String} LIMIT 1",
        {
            "src": scenario.source,
            "source": source,
            "data_source": data_source,
            "event_kind": event_kind,
            "field_id": field_id,
            "version": FAR_FUTURE_MS,
        },
    )


def _land_first_issue(scenario: Scenario) -> None:
    scenario.warehouse.execute(
        "DELETE FROM silver.class_task_field_history WHERE insight_source_id IN ({src:String}, 'another-connection')"
        " SETTINGS mutations_sync = 2",
        {"src": scenario.source},
    )
    scenario.seed(fields=[STATUS_FIELD], issues=[_issue("TST-1")], events=[_started("TST-1")], statuses=STATUSES)
    scenario.build()
    _build_class(scenario)
    assert _issues_in_class(scenario) == {"TST-1"}


def _land_second_issue(scenario: Scenario) -> None:
    scenario.seed(
        fields=[],
        issues=[_issue("TST-2", extracted_at=LATER_SYNC)],
        events=[_started("TST-2", extracted_at=LATER_SYNC)],
    )
    scenario.build(full_refresh=False)
    _build_class(scenario)


def test_another_connectors_newer_rows_do_not_hide_the_journal(scenario: Scenario) -> None:
    _land_first_issue(scenario)
    _plant_future_row(
        scenario, source="another-connection", data_source="github", event_kind="changelog", field_id="status"
    )

    _land_second_issue(scenario)

    assert _issues_in_class(scenario) == {"TST-1", "TST-2"}, "the journal's later issue must reach the class"


def test_a_sibling_producer_of_the_same_connection_does_not_hide_the_journal(scenario: Scenario) -> None:
    """The comment lifecycle arm shares the journal's connection and data source
    but stamps build time, so the boundary has to tell the two apart."""
    _land_first_issue(scenario)
    _plant_future_row(scenario, source=scenario.source, data_source="jira", event_kind="lifecycle", field_id="comment")

    _land_second_issue(scenario)

    assert _issues_in_class(scenario) == {"TST-1", "TST-2"}, "the journal's later issue must reach the class"
