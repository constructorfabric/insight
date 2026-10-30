import json

from conftest import SOURCE, TENANT, Warehouse

OLD_DESCRIPTION = "Old synthetic description " * 20
NEW_DESCRIPTION = "New synthetic description " * 20
OLD_NOTES = "Old synthetic notes " * 20
NEW_NOTES = "New synthetic notes " * 20
CLASSES = "class_task_field_history class_task_field_text"
CREATED = 1767225600000


def text_field_issue(case: Warehouse, description: str, notes: str, observed: str = "2026-01-10T00:00:00") -> None:
    case.insert(
        "youtrack_custom_fields",
        {"id": "field-1", "unique_key": "synthetic-field", "name": "Synthetic notes", "field_type_id": "text[1]"},
    )
    notes_field = {
        "id": "issue-field",
        "name": "Synthetic notes",
        "$type": "TextIssueCustomField",
        "projectCustomField": {"id": "project-field", "field": {"id": "field-1"}},
        "value": {"id": "text-value", "text": notes, "$type": "TextFieldValue"},
    }
    case.insert(
        "youtrack_issues",
        {
            "id": "issue-1",
            "idReadable": "EX-1",
            "created": CREATED,
            "unique_key": TENANT + "-" + SOURCE + "-issue-1",
            "reporter_id": "user-1",
            "project_id": "project-1",
            "custom_fields_json": json.dumps([notes_field]),
            "issue_json": json.dumps({"summary": "Synthetic issue", "description": description, "links": []}),
        },
        observed,
    )


def text_change(
    case: Warehouse, event_id: str, activity_type: str, target_member: str, before: str, after: str, at: int
) -> None:
    # The connector encodes a text value twice, as it does every activity value.
    case.insert(
        "youtrack_activities",
        {
            "id": event_id,
            "unique_key": TENANT + "-" + SOURCE + "-" + event_id,
            "_type": activity_type,
            "timestamp": str(at),
            "author_id": "user-1",
            "field_json": json.dumps({"id": "filter-1", "customField": {"id": "field-1"}, "name": "Synthetic notes"}),
            "target_json": '{"id":"issue-1","idReadable":"EX-1","$type":"Issue"}',
            "removed_json": json.dumps(json.dumps(before)),
            "added_json": json.dumps(json.dumps(after)),
            "activity_json": json.dumps({"targetMember": target_member}),
        },
    )


def seed(case: Warehouse) -> None:
    text_field_issue(case, NEW_DESCRIPTION, NEW_NOTES)
    # The feed starts at the issue's creation, so its initial values are known.
    text_change(case, "describe", "TextMarkupActivityItem", "description", OLD_DESCRIPTION, NEW_DESCRIPTION, CREATED)
    text_change(case, "annotate", "TextCustomFieldActivityItem", "", OLD_NOTES, NEW_NOTES, CREATED + 86400000)
    case.build()
    case.build(CLASSES)


def test_a_silver_only_query_reads_every_body_of_a_long_text_field(case: Warehouse) -> None:
    seed(case)
    rows = list(
        case.rows("""
        SELECT h.field_id AS field_id, h.event_kind AS event_kind, h.value_id_type AS value_id_type,
            h.value_displays AS value_displays, t.content_form AS content_form, t.content AS content
        FROM silver.class_task_field_history AS h FINAL
        LEFT JOIN silver.class_task_field_text AS t FINAL
            ON t.tenant_id = h.tenant_id AND t.insight_source_id = h.insight_source_id
           AND t.data_source = h.data_source
           AND t.text_id = h.value_ids[1]
        WHERE h.data_source = 'youtrack' AND h.field_id IN ('description', 'field-1')
          AND h.event_kind IN ('synthetic_initial', 'changelog')
        ORDER BY h.field_id, h.event_order""")
    )
    assert [(r["field_id"], r["event_kind"], r["content"]) for r in rows] == [
        ("description", "synthetic_initial", OLD_DESCRIPTION),
        ("description", "changelog", NEW_DESCRIPTION),
        ("field-1", "synthetic_initial", OLD_NOTES),
        ("field-1", "changelog", NEW_NOTES),
    ]
    for row in rows:
        assert row["value_id_type"] == "text_ref", f"should be a text reference: {row!r}"
        assert row["content_form"] == "markdown", f"should be the stored Markdown source: {row!r}"
        assert row["value_displays"] == [row["content"][:200]], f"should carry only a prefix: {row!r}"


def test_the_history_carries_no_long_text_body(case: Warehouse) -> None:
    seed(case)
    longest = list(
        case.rows("""
        SELECT max(length(arrayJoin(value_displays))) AS longest
        FROM silver.class_task_field_history FINAL
        WHERE data_source = 'youtrack' AND field_id IN ('description', 'field-1')""")
    )
    assert longest == [{"longest": 200}]


def test_an_empty_description_has_no_reference(case: Warehouse) -> None:
    text_field_issue(case, "", NEW_NOTES)
    case.build()
    rows = list(
        case.rows("""
        SELECT value_ids, value_displays FROM staging.youtrack__task_field_history
        WHERE field_id = 'description'""")
    )
    assert rows == [{"value_ids": [], "value_displays": []}]


def test_every_text_reference_resolves(case: Warehouse) -> None:
    seed(case)
    case.build("class_task_field_current_text")
    case.dbt("test", "--select", "assert_text_refs_resolve")


def test_the_current_body_is_found_by_the_issue_id(case: Warehouse) -> None:
    seed(case)
    case.build("class_task_field_current_text")
    rows = list(
        case.rows("""
        SELECT c.issue_id AS issue_id, c.field_id AS field_id, t.content AS content
        FROM silver.class_task_field_current_text AS c FINAL
        INNER JOIN silver.class_task_field_text AS t FINAL
            ON t.tenant_id = c.tenant_id AND t.insight_source_id = c.insight_source_id
           AND t.data_source = c.data_source
           AND t.text_id = c.text_id
        WHERE c.data_source = 'youtrack'
        ORDER BY c.field_id""")
    )
    assert rows == [
        {"issue_id": "issue-1", "field_id": "description", "content": NEW_DESCRIPTION},
        {"issue_id": "issue-1", "field_id": "field-1", "content": NEW_NOTES},
    ]


def test_an_emptied_field_has_no_current_body(case: Warehouse) -> None:
    seed(case)
    case.build("class_task_field_current_text")
    text_field_issue(case, "", NEW_NOTES, "2026-01-11T00:00:00")
    case.build()
    case.build("class_task_field_current_text")
    rows = list(
        case.rows("""
        SELECT field_id FROM silver.class_task_field_current_text FINAL
        WHERE data_source = 'youtrack' ORDER BY field_id""")
    )
    assert rows == [{"field_id": "field-1"}]
