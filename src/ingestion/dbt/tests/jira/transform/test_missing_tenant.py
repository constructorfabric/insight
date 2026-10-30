"""A bronze row without a tenant builds, and the tenant tests on the model that reads it report it.

A key reads the tenant through `ifNull(tenant_id, '')`, so a missing one never makes a key
Nullable; `not_null` and `not_empty` on each model's tenant are the guard. Each case builds
the chain from rows that carry a tenant, adds one row that does not, and rebuilds only the
model that reads it, so the finding is that model's own.
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
SELECTOR = (
    f"{FIELD_HISTORY_SELECTOR} +jira__task_links {LIFECYCLE_SELECTOR} +jira__task_field_metadata"
    " +jira__task_field_current_text"
)
CLASSES = (
    "class_task_field_history",
    "class_task_links",
    "class_task_field_metadata",
    "class_task_field_text",
    "class_task_field_current_text",
)
PRODUCERS = (
    "jira__field_history_derived",
    "jira__task_links",
    "jira__comment_lifecycle_events",
    "jira__worklog_lifecycle_events",
    "jira__task_field_metadata",
    "jira__task_field_text",
    "jira__task_field_current_text",
)

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
    "jira__task_field_metadata": ("bronze_jira.jira_fields", lambda: field("customfield_10097", schema_type="number")),
    "jira__issue_key_ids": ("bronze_jira.jira_issue", _issue),
    "jira__changelog_items": (
        "bronze_jira.jira_issue_history",
        lambda: event("TST-9", 909, "2026-01-07T10:00:00", [item(POINTS, frm="1", frm_str="1", to="2", to_str="2")]),
    ),
    "jira__issue_availability_state": ("bronze_jira.jira_issue", _issue),
    "jira__task_field_kind": ("bronze_jira.jira_fields", lambda: field("customfield_10099", schema_type="number")),
    "jira__comment_state": (
        "bronze_jira.jira_comments",
        lambda: comment("TST-9", 909, jira_id="TST-9", updated="2026-01-07T10:00:00.000+0000"),
    ),
}

REPORTED_BY = {
    "jira__catalogue_first_seen": "not_null_jira__catalogue_first_seen_tenant_id",
    "jira__changelog_items": "not_empty_jira__changelog_items_tenant_id",
    "jira__comment_state": "not_null_jira__comment_state_tenant_id",
    "jira__issue_availability_state": "not_null_jira__issue_availability_state_tenant_id",
    "jira__issue_key_ids": "not_null_jira__issue_key_ids_tenant_id",
    "jira__task_field_kind": "not_null_jira__task_field_kind_tenant_id",
    "jira__task_field_metadata": "not_empty_jira__task_field_metadata_tenant_id",
}


def tenant_tests(model: str) -> tuple[str, str]:
    return f"not_null_{model}_tenant_id", f"not_empty_{model}_tenant_id"


def _rebuild(warehouse: Warehouse, model: str) -> None:
    if model == "jira__catalogue_first_seen":
        warehouse.execute("TRUNCATE TABLE IF EXISTS staging.jira__catalogue_first_seen")
        warehouse.dbt("run", "--select", model)
        return
    warehouse.dbt("run", "--select", model, "--full-refresh")


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
def test_a_bronze_row_without_a_tenant_builds_and_the_model_that_reads_it_reports_it(
    built: Warehouse, model: str
) -> None:
    table, row = CASES[model]
    assert built.failing_tests(tenant_tests(model)) == frozenset(), f"{model} should start from tenanted rows only"

    built.insert(table, [_without_tenant(row())])
    try:
        invocation = built.dbt_status("run", "--select", model)
        failing = built.failing_tests(tenant_tests(model))
    finally:
        built.execute(DROP_TENANTLESS[table])
        _rebuild(built, model)

    assert invocation.success, f"{model} should build over a {table} row without a tenant:\n{invocation.log}"
    assert REPORTED_BY[model] in failing, f"{model} should report the {table} row without a tenant: {sorted(failing)}"


def test_a_tenantless_issue_reaches_the_journal_and_its_tenant_tests_report_it(built: Warehouse) -> None:
    rows = {
        "bronze_jira.jira_issue": _issue(),
        "bronze_jira.jira_issue_history": event(
            "TST-9",
            909,
            "2026-01-07T10:00:00",
            [item(POINTS, frm="1", frm_str="1", to="2", to_str="2")],
            jira_id="9009",
        ),
    }
    for table, row in rows.items():
        built.insert(table, [_without_tenant(row)])
    try:
        built.build(SELECTOR)
        failing = built.failing_tests(tenant_tests("jira__field_history_derived"))
    finally:
        for table in rows:
            built.execute(DROP_TENANTLESS[table])
        built.build(SELECTOR)

    assert "not_empty_jira__field_history_derived_tenant_id" in failing, (
        f"the journal should report it: {sorted(failing)}"
    )


def test_the_producers_and_their_classes_carry_a_string_tenant(built: Warehouse) -> None:
    built.dbt("run", "--select", *CLASSES, "--full-refresh")

    types = {
        f"{row['database']}.{row['table']}": row["type"]
        for row in built.rows(
            "SELECT database, table, type FROM system.columns WHERE name = 'tenant_id'"
            " AND ((database = 'staging' AND table IN {producers:Array(String)})"
            " OR (database = 'silver' AND table IN {classes:Array(String)}))",
            {"producers": list(PRODUCERS), "classes": list(CLASSES)},
        )
    }

    expected = {f"staging.{name}": "String" for name in PRODUCERS} | {f"silver.{name}": "String" for name in CLASSES}
    assert types == expected, f"every producer and class should carry a String tenant: {types}"
