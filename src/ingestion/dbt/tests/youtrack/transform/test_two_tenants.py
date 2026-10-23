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


def _issue(
    case: Warehouse,
    tenant: str,
    value: object,
    field_type: str = "MultiEnumIssueCustomField",
    observed: str = "2026-01-10T00:00:00",
) -> None:
    field = {
        "id": "issue-field",
        "name": "Synthetic field",
        "$type": field_type,
        "projectCustomField": {"id": "project-field", "field": {"id": "field-1"}},
        "value": value,
    }
    case.insert(
        "youtrack_issues",
        {
            "tenant_id": tenant,
            "id": "issue-1",
            "idReadable": "EX-1",
            "created": 1767225600000,
            "unique_key": f"{tenant}-{SOURCE}-issue-1",
            "reporter_id": "user-1",
            "project_id": "project-1",
            "custom_fields_json": json.dumps([field]),
            "issue_json": json.dumps({"summary": "Synthetic issue", "description": "", "links": []}),
        },
        observed,
    )


def _activity(case: Warehouse, tenant: str, event_id: str, value_id: str, at: int = 1767312000000) -> None:
    case.insert(
        "youtrack_activities",
        {
            "tenant_id": tenant,
            "id": event_id,
            "unique_key": f"{tenant}-{SOURCE}-{event_id}",
            "_type": "CustomFieldActivityItem",
            "timestamp": str(at),
            "author_id": "user-1",
            "field_json": json.dumps({"id": "filter-1", "customField": {"id": "field-1"}, "name": "Synthetic field"}),
            "target_json": json.dumps({"id": "issue-1", "idReadable": "EX-1", "$type": "Issue"}),
            "added_json": json.dumps(_value(value_id)),
            "removed_json": json.dumps([]),
            "activity_json": "{}",
        },
    )


def _field_type(case: Warehouse, tenant: str, field_type: str, observed: str) -> None:
    case.insert(
        "youtrack_custom_fields",
        {
            "tenant_id": tenant,
            "id": "field-1",
            "unique_key": f"{tenant}-{SOURCE}-field-1",
            "name": "Synthetic field",
            "field_type_id": field_type,
        },
        observed,
    )


def _seed(case: Warehouse) -> None:
    for tenant, value_id in [(TENANT, "mine"), (OTHER_TENANT, "theirs")]:
        _issue(case, tenant, _value(value_id))
        _activity(case, tenant, "activity-1", value_id)
    case.build()


# INVARIANT: the other tenant's type is observed after this tenant's, so a type
# match that ignores the tenant picks the foreign type for every later instant.
FIELD_TYPES = [(TENANT, "enum[1]", "2026-01-10T00:00:00"), (OTHER_TENANT, "enum[*]", "2026-01-11T00:00:00")]
BEFORE_ANY_TYPE = 1767312000000
AFTER_EVERY_TYPE = 1768176000000


def _seed_field_types(case: Warehouse) -> None:
    for tenant, field_type, observed in FIELD_TYPES:
        _field_type(case, tenant, field_type, observed)
        _activity(case, tenant, "before-any-type", "a", BEFORE_ANY_TYPE)
        _activity(case, tenant, "after-every-type", "b", AFTER_EVERY_TYPE)
        _issue(
            case,
            tenant,
            {"id": "a", "name": "A", "$type": "StateBundleElement"},
            "StateIssueCustomField",
            "2026-01-05T00:00:00",
        )
    case.build()
    for tenant, _, _ in FIELD_TYPES:
        _issue(
            case,
            tenant,
            {"id": "b", "name": "B", "$type": "StateBundleElement"},
            "StateIssueCustomField",
            "2026-01-12T00:00:00",
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


def test_each_tenant_types_its_events_by_its_own_field_types(case: Warehouse) -> None:
    _seed_field_types(case)

    found = case.rows(
        "SELECT tenant_id, event_id, field_cardinality FROM staging.youtrack__activities"
        f" WHERE insight_source_id = '{SOURCE}' AND field_id = 'field-1' ORDER BY tenant_id, event_id"
    )

    assert list(found) == [
        {"tenant_id": OTHER_TENANT, "event_id": "after-every-type", "field_cardinality": "multi"},
        {"tenant_id": OTHER_TENANT, "event_id": "before-any-type", "field_cardinality": "multi"},
        {"tenant_id": TENANT, "event_id": "after-every-type", "field_cardinality": "single"},
        {"tenant_id": TENANT, "event_id": "before-any-type", "field_cardinality": "single"},
    ]


def test_each_tenant_types_its_snapshots_by_its_own_field_types(case: Warehouse) -> None:
    _seed_field_types(case)

    found = case.rows(
        "SELECT tenant_id, toString(observed_at) AS observed_at, field_cardinality"
        " FROM staging.youtrack__issue_field_snapshot"
        f" WHERE insight_source_id = '{SOURCE}' AND field_id = 'field-1' ORDER BY tenant_id, observed_at"
    )

    assert list(found) == [
        {"tenant_id": OTHER_TENANT, "observed_at": "2026-01-05 00:00:00.000", "field_cardinality": "multi"},
        {"tenant_id": OTHER_TENANT, "observed_at": "2026-01-12 00:00:00.000", "field_cardinality": "multi"},
        {"tenant_id": TENANT, "observed_at": "2026-01-05 00:00:00.000", "field_cardinality": "single"},
        {"tenant_id": TENANT, "observed_at": "2026-01-12 00:00:00.000", "field_cardinality": "single"},
    ]


def test_each_tenant_keeps_its_own_field_type_history(case: Warehouse) -> None:
    _seed_field_types(case)

    found = case.rows(
        "SELECT tenant_id, observed_cardinality, previously_observed_type FROM staging.youtrack__field_type_history"
        f" WHERE insight_source_id = '{SOURCE}' AND field_id = 'field-1' ORDER BY tenant_id"
    )

    assert list(found) == [
        {"tenant_id": OTHER_TENANT, "observed_cardinality": "multi", "previously_observed_type": None},
        {"tenant_id": TENANT, "observed_cardinality": "single", "previously_observed_type": None},
    ]


THEIR_ENUM = "Synthetic enum value of the other tenant"


def _seed_long_text_type_for_one_tenant(case: Warehouse) -> None:
    _field_type(case, TENANT, "text[1]", "2026-01-10T00:00:00")
    _field_type(case, OTHER_TENANT, "enum[1]", "2026-01-10T00:00:00")
    _issue(
        case,
        TENANT,
        {"id": "text-value", "text": "Synthetic notes body", "$type": "TextFieldValue"},
        "TextIssueCustomField",
    )
    _issue(
        case,
        OTHER_TENANT,
        {"id": "theirs", "name": THEIR_ENUM, "$type": "EnumBundleElement"},
        "SingleEnumIssueCustomField",
    )
    case.build()


def test_a_field_is_long_text_only_for_the_tenant_that_types_it_so(case: Warehouse) -> None:
    _seed_long_text_type_for_one_tenant(case)

    found = case.rows(
        "SELECT tenant_id, field_id FROM staging.youtrack__long_text_fields"
        f" WHERE insight_source_id = '{SOURCE}' ORDER BY tenant_id, field_id"
    )

    assert list(found) == [
        {"tenant_id": OTHER_TENANT, "field_id": "description"},
        {"tenant_id": TENANT, "field_id": "description"},
        {"tenant_id": TENANT, "field_id": "field-1"},
    ]


def test_another_tenants_value_of_the_same_field_is_never_stored_as_a_body(case: Warehouse) -> None:
    _seed_long_text_type_for_one_tenant(case)

    bodies = [
        str(row["content"])
        for row in case.rows(
            f"SELECT content FROM staging.youtrack__task_field_text WHERE insight_source_id = '{SOURCE}'"
        )
    ]

    assert "Synthetic notes body" in bodies, bodies
    assert THEIR_ENUM not in bodies, bodies


SHARED_NOTES = "Shared synthetic notes " * 20
DESCRIPTIONS = {
    TENANT: "Synthetic description of this tenant " * 10,
    OTHER_TENANT: "Synthetic description of theirs " * 10,
}
TEXT_CLASSES = "class_task_field_history class_task_field_text class_task_field_current_text"


def _seed_text_classes(case: Warehouse) -> None:
    for tenant, description in DESCRIPTIONS.items():
        _field_type(case, tenant, "text[1]", "2026-01-10T00:00:00")
        notes = {
            "id": "issue-field",
            "name": "Synthetic field",
            "$type": "TextIssueCustomField",
            "projectCustomField": {"id": "project-field", "field": {"id": "field-1"}},
            "value": {"id": "text-value", "text": SHARED_NOTES, "$type": "TextFieldValue"},
        }
        case.insert(
            "youtrack_issues",
            {
                "tenant_id": tenant,
                "id": "issue-1",
                "idReadable": "EX-1",
                "created": 1767225600000,
                "unique_key": f"{tenant}-{SOURCE}-issue-1",
                "reporter_id": "user-1",
                "project_id": "project-1",
                "custom_fields_json": json.dumps([notes]),
                "issue_json": json.dumps({"summary": "Synthetic issue", "description": description, "links": []}),
            },
        )
    case.build()
    case.build(TEXT_CLASSES)


def test_the_same_body_is_stored_once_per_tenant_in_the_class(case: Warehouse) -> None:
    _seed_text_classes(case)

    rows = list(
        case.rows(
            "SELECT tenant_id, text_id, unique_key FROM silver.class_task_field_text FINAL"
            f" WHERE data_source = 'youtrack' AND insight_source_id = '{SOURCE}'"
            f" AND startsWith(content, 'Shared synthetic notes') ORDER BY tenant_id"
        )
    )

    assert [r["tenant_id"] for r in rows] == sorted(DESCRIPTIONS), rows
    assert len({r["text_id"] for r in rows}) == 1, f"one body should hash to one text_id: {rows}"
    assert all(str(r["unique_key"]).startswith(f"{r['tenant_id']}-{SOURCE}-youtrack-") for r in rows), rows


def test_each_tenants_current_description_resolves_to_its_own_body(case: Warehouse) -> None:
    _seed_text_classes(case)

    found = _per_tenant(
        case,
        "SELECT c.tenant_id AS tenant_id, t.content AS value"
        " FROM silver.class_task_field_current_text AS c FINAL"
        " INNER JOIN silver.class_task_field_text AS t FINAL"
        "     ON t.tenant_id = c.tenant_id AND t.insight_source_id = c.insight_source_id"
        "    AND t.data_source = c.data_source AND t.text_id = c.text_id"
        f" WHERE c.data_source = 'youtrack' AND c.insight_source_id = '{SOURCE}'"
        " AND c.issue_id = 'issue-1' AND c.field_id = 'description'",
    )

    assert found == DESCRIPTIONS


def test_every_text_reference_of_two_tenants_resolves(case: Warehouse) -> None:
    _seed_text_classes(case)

    case.dbt("test", "--select", "assert_text_refs_resolve")
