"""Two tenants that share a source id, issue ids and activity ids stay apart.

A source id names a connector instance inside one tenant, and YouTrack ids are
unique only inside one instance, so two tenants can hold the very same ids with
different contents. The models up to the class boundary key, join, group and
order by tenant first.
"""

from __future__ import annotations

import json

from conftest import SOURCE, TENANT, Warehouse

OTHER_TENANT = "synthetic-other-tenant"


def _value(value_id: str) -> list[dict[str, str]]:
    return [{"id": value_id, "name": value_id.upper(), "$type": "EnumBundleElement"}]


def _seed(case: Warehouse) -> None:
    case.issue(_value("mine"))
    case.event("activity-1", [], _value("mine"))

    field = {
        "id": "issue-field",
        "name": "Synthetic field",
        "$type": "MultiEnumIssueCustomField",
        "projectCustomField": {"id": "project-field", "field": {"id": "field-1"}},
        "value": _value("theirs"),
    }
    case.insert(
        "youtrack_issues",
        {
            "tenant_id": OTHER_TENANT,
            "id": "issue-1",
            "idReadable": "EX-1",
            "created": 1767225600000,
            "unique_key": f"{OTHER_TENANT}-{SOURCE}-issue-1",
            "reporter_id": "user-1",
            "project_id": "project-1",
            "custom_fields_json": json.dumps([field]),
            "issue_json": json.dumps({"summary": "Synthetic issue", "description": "", "links": []}),
        },
    )
    case.insert(
        "youtrack_activities",
        {
            "tenant_id": OTHER_TENANT,
            "id": "activity-1",
            "unique_key": f"{OTHER_TENANT}-{SOURCE}-activity-1",
            "_type": "CustomFieldActivityItem",
            "timestamp": "1767312000000",
            "author_id": "user-1",
            "field_json": json.dumps({"id": "filter-1", "customField": {"id": "field-1"}, "name": "Synthetic field"}),
            "target_json": json.dumps({"id": "issue-1", "idReadable": "EX-1", "$type": "Issue"}),
            "added_json": json.dumps(_value("theirs")),
            "removed_json": json.dumps([]),
            "activity_json": "{}",
        },
    )
    case.build()


def _per_tenant(case: Warehouse, sql: str) -> dict[str, str]:
    return {str(row["tenant_id"]): str(row["value"]) for row in case.rows(sql)}


def test_each_tenant_keeps_its_own_activity(case: Warehouse) -> None:
    _seed(case)

    found = _per_tenant(
        case,
        "SELECT tenant_id, toString(after_pairs) AS value FROM staging.youtrack__activities"
        f" WHERE insight_source_id = '{SOURCE}' AND event_id = 'activity-1'",
    )

    assert set(found) == {TENANT, OTHER_TENANT}, found
    assert "mine" in found[TENANT] and "theirs" not in found[TENANT], found
    assert "theirs" in found[OTHER_TENANT] and "mine" not in found[OTHER_TENANT], found


def test_each_tenant_snapshots_its_own_issue(case: Warehouse) -> None:
    _seed(case)

    found = _per_tenant(
        case,
        "SELECT tenant_id, toString(pairs) AS value FROM staging.youtrack__issue_field_snapshot"
        f" WHERE insight_source_id = '{SOURCE}' AND issue_id = 'issue-1' AND field_id = 'field-1'",
    )

    assert set(found) == {TENANT, OTHER_TENANT}, found
    assert "mine" in found[TENANT] and "theirs" in found[OTHER_TENANT], found


def test_each_tenant_derives_its_own_field_states(case: Warehouse) -> None:
    _seed(case)

    states = list(
        case.rows(
            "SELECT tenant_id, toString(pairs) AS value FROM staging.youtrack__field_states"
            f" WHERE insight_source_id = '{SOURCE}' AND issue_id = 'issue-1' ORDER BY tenant_id"
        )
    )

    tenants = {str(row["tenant_id"]) for row in states}
    assert tenants == {TENANT, OTHER_TENANT}, states
    for row in states:
        foreign = "theirs" if row["tenant_id"] == TENANT else "mine"
        assert foreign not in str(row["value"]), f"{row['tenant_id']} reads the other tenant's value: {row}"
