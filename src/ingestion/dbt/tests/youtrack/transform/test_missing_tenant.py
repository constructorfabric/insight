"""A bronze row without a tenant stops the build instead of landing under an empty one.

Every YouTrack model that reads a bronze tenant converts it with
`CAST(tenant_id AS String)`, which raises on NULL.
"""

from __future__ import annotations

import json

import pytest
from conftest import SOURCE, Warehouse

NULL_TO_STRING = "Cannot convert NULL value to non-Nullable type"

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


@pytest.mark.parametrize("stream", sorted(TENANTLESS_ROWS))
def test_a_bronze_row_without_a_tenant_fails_the_build(case: Warehouse, stream: str) -> None:
    case.issue([])
    row = {**TENANTLESS_ROWS[stream], "tenant_id": None, "unique_key": f"{SOURCE}-no-tenant-{stream}"}
    case.insert(stream, row)

    with pytest.raises(AssertionError, match=NULL_TO_STRING):
        case.build()
