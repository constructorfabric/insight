"""A bronze row without a tenant builds, and the tenant tests on its models report it.

A key reads the tenant through `ifNull(tenant_id, '')`, so a missing one never makes
a sort key Nullable; `not_null` and `not_empty` on each model's tenant are the guard.
"""

from __future__ import annotations

import json

import pytest
from conftest import SOURCE, Warehouse

TENANT_MODELS = (
    "youtrack__activities",
    "youtrack__activity_order",
    "youtrack__bundle_values",
    "youtrack__comment_issues",
    "youtrack__field_states",
    "youtrack__field_type_history",
    "youtrack__issue_field_snapshot",
    "youtrack__issues",
    "youtrack__lifecycle_events",
    "youtrack__long_text_fields",
    "youtrack__projects",
)
TENANT_TESTS = tuple(f"{test}_{model}_tenant_id" for model in TENANT_MODELS for test in ("not_null", "not_empty"))
SORT_KEYED = (
    "youtrack__activities",
    "youtrack__activity_order",
    "youtrack__field_states",
    "youtrack__issue_field_snapshot",
)

TENANTLESS_ROWS: dict[str, dict[str, object]] = {
    "youtrack_issues": {
        "id": "issue-9",
        "idReadable": "EX-9",
        "created": 1767225600000,
        "reporter_id": "user-1",
        "project_id": "project-1",
        "custom_fields_json": "[]",
        "issue_json": json.dumps({"summary": "Synthetic issue", "description": "", "links": []}),
    },
    "youtrack_activities": {
        "id": "activity-9",
        "_type": "CustomFieldActivityItem",
        "timestamp": "1767312000000",
        "author_id": "user-1",
        "field_json": json.dumps({"id": "filter-1", "customField": {"id": "field-1"}, "name": "Synthetic field"}),
        "target_json": json.dumps({"id": "issue-1", "idReadable": "EX-1", "$type": "Issue"}),
        "added_json": "[]",
        "removed_json": "[]",
        "activity_json": "{}",
    },
    "youtrack_custom_fields": {"id": "field-9", "name": "Synthetic field", "field_type_id": "enum[1]"},
    "youtrack_comments": {
        "id": "comment-9",
        "issue_id": "issue-1",
        "issue_id_readable": "EX-1",
        "created": 1767312000000,
        "deleted": False,
        "author_id": "user-1",
        "text": "Synthetic comment",
    },
    "youtrack_work_items": {
        "id": "work-9",
        "issue_id": "issue-1",
        "author_id": "user-1",
        "date": 1767312000000,
        "work_item_json": '{"duration":{"minutes":15}}',
    },
}


FLAGGED_BY_STREAM: dict[str, set[str]] = {
    "youtrack_activities": {
        "not_empty_youtrack__activities_tenant_id",
        "not_empty_youtrack__activity_order_tenant_id",
        "not_empty_youtrack__field_states_tenant_id",
    },
    "youtrack_comments": {"not_null_youtrack__lifecycle_events_tenant_id"},
    "youtrack_custom_fields": {"not_null_youtrack__field_type_history_tenant_id"},
    "youtrack_issues": {
        "not_empty_youtrack__issues_tenant_id",
        "not_empty_youtrack__issue_field_snapshot_tenant_id",
        "not_empty_youtrack__long_text_fields_tenant_id",
    },
    "youtrack_work_items": {"not_null_youtrack__lifecycle_events_tenant_id"},
}


def test_the_tenant_tests_pass_over_rows_that_carry_a_tenant(case: Warehouse) -> None:
    case.issue([])
    for stream, row in TENANTLESS_ROWS.items():
        case.insert(stream, {**row, "unique_key": f"{SOURCE}-{stream}"})
    case.build()

    assert case.failing_tests(TENANT_TESTS) == set(), "no tenant test should fail while every row carries a tenant"


@pytest.mark.parametrize("stream", sorted(TENANTLESS_ROWS))
def test_a_bronze_row_without_a_tenant_builds_and_the_tenant_tests_report_it(case: Warehouse, stream: str) -> None:
    case.issue([])
    row = {**TENANTLESS_ROWS[stream], "tenant_id": None, "unique_key": f"{SOURCE}-no-tenant-{stream}"}
    case.insert(stream, row)
    case.build()

    failing = case.failing_tests(TENANT_TESTS)
    assert FLAGGED_BY_STREAM[stream] <= failing, f"the tenant tests should report the {stream} row: {sorted(failing)}"


def test_the_sort_keyed_relations_carry_a_string_tenant(case: Warehouse) -> None:
    case.issue([])
    case.build(full_refresh=True)

    types = {
        row["table"]: row["type"]
        for row in case.rows("SELECT table, type FROM system.columns WHERE database = 'staging' AND name = 'tenant_id'")
        if row["table"] in SORT_KEYED
    }

    assert types == dict.fromkeys(SORT_KEYED, "String"), f"a sort key should never hold a Nullable tenant: {types}"


PRODUCERS = (
    "youtrack__task_field_history",
    "youtrack__task_field_metadata",
    "youtrack__task_issuetypes",
    "youtrack__task_links",
    "youtrack__task_comments",
    "youtrack__task_projects",
    "youtrack__task_sprints",
    "youtrack__task_statuses",
    "youtrack__task_worklogs",
    "youtrack__task_field_text",
    "youtrack__task_field_current_text",
)


def test_every_task_producer_carries_a_string_tenant(case: Warehouse) -> None:
    case.issue([])
    case.build(full_refresh=True)

    types = {
        row["table"]: row["type"]
        for row in case.rows(
            "SELECT table, type FROM system.columns WHERE database = 'staging' AND name = 'tenant_id'"
            " AND table LIKE 'youtrack\\_\\_task\\_%' AND table != 'youtrack__task_users'"
        )
    }

    assert types == dict.fromkeys(PRODUCERS, "String"), f"every producer should carry a String tenant: {types}"
