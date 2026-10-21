"""A bronze row without a tenant stops the build instead of landing under an empty one.

Every model that reads a bronze tenant converts it with `CAST(tenant_id AS String)`,
which raises on NULL. Each case builds the chain from rows that carry a tenant, adds
one row that does not, and rebuilds only the model that reads it, so the failure is
that model's own. A view raises when its tenant is read instead.
"""

from __future__ import annotations

from collections.abc import Callable, Iterator
from typing import Any

import pytest
from conftest import FIELD_HISTORY_SELECTOR, LIFECYCLE_SELECTOR, Scenario, Warehouse, _truncate_bronze
from helpers import comment, event, field, issue, item, worklog

POINTS = "customfield_10001"
NOTES = "customfield_10010"
TEXTAREA = "com.atlassian.jira.plugin.system.customfieldtypes:textarea"
SELECTOR = f"{FIELD_HISTORY_SELECTOR} +jira__task_links {LIFECYCLE_SELECTOR}"
NULL_TO_STRING = "Cannot convert NULL value to non-Nullable type"

# Spelled out: a table name cannot be bound, and a formatted statement is what the security gate rejects.
DROP_TENANTLESS = {
    "bronze_jira.jira_fields": "ALTER TABLE bronze_jira.jira_fields DELETE WHERE tenant_id IS NULL SETTINGS mutations_sync = 2",
    "bronze_jira.jira_issue": "ALTER TABLE bronze_jira.jira_issue DELETE WHERE tenant_id IS NULL SETTINGS mutations_sync = 2",
    "bronze_jira.jira_issue_history": (
        "ALTER TABLE bronze_jira.jira_issue_history DELETE WHERE tenant_id IS NULL SETTINGS mutations_sync = 2"
    ),
    "bronze_jira.jira_comments": (
        "ALTER TABLE bronze_jira.jira_comments DELETE WHERE tenant_id IS NULL SETTINGS mutations_sync = 2"
    ),
    "bronze_jira.jira_worklogs": (
        "ALTER TABLE bronze_jira.jira_worklogs DELETE WHERE tenant_id IS NULL SETTINGS mutations_sync = 2"
    ),
}


def _without_tenant(record: dict[str, Any]) -> dict[str, Any]:
    return {**record, "tenant_id": None, "unique_key": f"no-tenant-{record['unique_key']}"}


def _issue() -> dict[str, Any]:
    return issue("TST-9", jira_id="9009", fields={POINTS: 1, NOTES: "body"})


# model -> (the bronze table it reads, a row of that table)
CASES: dict[str, tuple[str, Callable[[], dict[str, Any]]]] = {
    "jira__catalogue_first_seen": ("bronze_jira.jira_fields", lambda: field("customfield_10098", schema_type="number")),
    "jira__issue_key_ids": ("bronze_jira.jira_issue", _issue),
    "jira__issue_field_snapshot": ("bronze_jira.jira_issue", _issue),
    "jira__issue_link_snapshot": ("bronze_jira.jira_issue", _issue),
    "jira__task_field_text": ("bronze_jira.jira_issue", _issue),
    "jira__changelog_items": (
        "bronze_jira.jira_issue_history",
        lambda: event("TST-9", 909, "2026-01-07T10:00:00", [item(POINTS, frm="1", frm_str="1", to="2", to_str="2")]),
    ),
    "jira__issue_availability_state": ("bronze_jira.jira_issue", _issue),
    "jira__comment_state": (
        "bronze_jira.jira_comments",
        lambda: comment("TST-9", 909, jira_id="TST-9", updated="2026-01-07T10:00:00.000+0000"),
    ),
    "jira__worklog_state": (
        "bronze_jira.jira_worklogs",
        lambda: worklog("TST-9", 909, jira_id="TST-9", updated="2026-01-07T10:00:00.000+0000"),
    ),
    "jira__comment_lifecycle_events": (
        "bronze_jira.jira_comments",
        lambda: comment("TST-9", 909, jira_id="TST-9", updated="2026-01-07T10:00:00.000+0000"),
    ),
    "jira__worklog_lifecycle_events": (
        "bronze_jira.jira_worklogs",
        lambda: worklog("TST-9", 909, jira_id="TST-9", updated="2026-01-07T10:00:00.000+0000"),
    ),
}


@pytest.fixture(scope="module")
def built(warehouse: Warehouse) -> Iterator[Warehouse]:
    _truncate_bronze(warehouse)
    Scenario(warehouse).seed(
        fields=[
            field(POINTS, name="Story Points", schema_type="number"),
            field(NOTES, name="Notes", schema_type="string", schema_custom=TEXTAREA),
        ],
        issues=[issue("TST-1", fields={POINTS: 5, NOTES: "body"})],
        events=[event("TST-1", 101, "2026-01-06T10:00:00", [item(POINTS, frm="3", frm_str="3", to="5", to_str="5")])],
        comments=[comment("TST-1", 501, jira_id="TST-1", updated="2026-01-06T10:00:00.000+0000")],
        worklogs=[worklog("TST-1", 601, jira_id="TST-1", updated="2026-01-06T10:00:00.000+0000")],
    )
    warehouse.build(SELECTOR)
    yield warehouse
    _truncate_bronze(warehouse)


@pytest.mark.parametrize("model", sorted(CASES))
def test_a_bronze_row_without_a_tenant_fails_the_model_that_reads_it(built: Warehouse, model: str) -> None:
    table, row = CASES[model]
    built.insert(table, [_without_tenant(row())])
    try:
        invocation = built.dbt_status("run", "--select", model)
    finally:
        built.execute(DROP_TENANTLESS[table])

    assert not invocation.success, f"{model} should refuse a {table} row without a tenant"
    assert NULL_TO_STRING in invocation.log, f"{model} should fail on the NULL tenant itself:\n{invocation.log}"


def test_the_field_kind_view_refuses_a_catalogue_row_without_a_tenant(built: Warehouse) -> None:
    built.insert("bronze_jira.jira_fields", [_without_tenant(field("customfield_10099", schema_type="number"))])
    try:
        with pytest.raises(Exception, match=NULL_TO_STRING):
            built.rows("SELECT tenant_id FROM staging.jira__task_field_kind")
    finally:
        built.execute(DROP_TENANTLESS["bronze_jira.jira_fields"])
