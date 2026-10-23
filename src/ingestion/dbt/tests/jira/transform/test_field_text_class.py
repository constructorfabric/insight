"""Long-text bodies reach silver: `class_task_field_history` refers to a body by
a text reference, and `class_task_field_text` holds it, so a silver-only query
reads the full text of a current or a historical value."""

from __future__ import annotations

import json
from collections.abc import Iterator

import pytest
from conftest import FIELD_HISTORY_SELECTOR, Scenario
from helpers import event, field, issue, item

DESCRIPTION_ADF = {
    "type": "doc",
    "version": 1,
    "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Synthetic body " * 40}]}],
}
OLD_BODY = "Old synthetic body " * 20
NEW_BODY = "New synthetic body " * 20
ISSUE_ID = "10001"

CLASSES = ("class_task_field_history", "class_task_field_text", "class_task_field_current_text")


CLEAR_CLASSES = (
    "DELETE FROM silver.class_task_field_history WHERE insight_source_id = {src:String} SETTINGS mutations_sync = 2",
    "DELETE FROM silver.class_task_field_text WHERE insight_source_id = {src:String} SETTINGS mutations_sync = 2",
    "DELETE FROM silver.class_task_field_current_text WHERE insight_source_id = {src:String} SETTINGS mutations_sync = 2",
)


def _clear_classes(scenario: Scenario) -> None:
    for statement in CLEAR_CLASSES:
        scenario.warehouse.execute(statement, {"src": scenario.source})


@pytest.fixture
def silver(scenario: Scenario) -> Iterator[Scenario]:
    scenario.seed(
        fields=[field("description", name="Description", schema_type="string")],
        issues=[issue("TST-1", fields={"description": DESCRIPTION_ADF}, jira_id=ISSUE_ID)],
        events=[
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item("description", frm_str=OLD_BODY, to_str=NEW_BODY)],
                jira_id=ISSUE_ID,
            )
        ],
    )
    scenario.build(f"{FIELD_HISTORY_SELECTOR} +jira__task_field_current_text")
    # Incremental on purpose: a full refresh would drop rows other modules seed.
    scenario.warehouse.dbt("run", "--select", *CLASSES)
    yield scenario
    _clear_classes(scenario)


def _bodies_by_event(scenario: Scenario) -> list[dict[str, object]]:
    return scenario.warehouse.rows(
        "SELECT h.event_kind AS event_kind, h.value_id_type AS value_id_type, h.value_displays AS value_displays,"
        "       t.content_form AS content_form, t.content AS content"
        " FROM silver.class_task_field_history AS h FINAL"
        " LEFT JOIN silver.class_task_field_text AS t FINAL"
        "        ON t.tenant_id = h.tenant_id"
        "       AND t.insight_source_id = h.insight_source_id"
        "       AND t.data_source = h.data_source"
        "       AND t.text_id = h.value_ids[1]"
        " WHERE h.insight_source_id = {src:String} AND h.field_id = 'description'"
        " ORDER BY h.event_order",
        {"src": scenario.source},
    )


def test_a_silver_only_query_reads_every_body_of_the_history(silver: Scenario) -> None:
    rows = _bodies_by_event(silver)

    assert [(r["event_kind"], r["content_form"], r["content"]) for r in rows] == [
        ("synthetic_initial", "rendered_text", OLD_BODY),
        ("changelog", "rendered_text", NEW_BODY),
    ]
    assert {r["value_id_type"] for r in rows} == {"text_ref"}
    for row in rows:
        prefix = row["value_displays"][0]
        assert len(prefix) < len(row["content"]), f"the display should be a prefix only: {prefix!r}"


def test_the_current_body_is_the_issue_document(silver: Scenario) -> None:
    current = silver.text_rows()
    adf = [r for r in current if r["content_form"] == "adf_json"]

    assert len(adf) == 1
    assert json.loads(adf[0]["content"]) == DESCRIPTION_ADF
    assert {r["data_source"] for r in current} == {"jira"}


def test_every_text_reference_resolves(silver: Scenario) -> None:
    """`assert_text_refs_resolve`, scoped to this scenario: the class tables
    outlive the module and hold other scenarios' references."""
    unresolved = silver.warehouse.rows(
        "SELECT h.event_id AS event_id, ref AS text_id"
        " FROM silver.class_task_field_history AS h FINAL"
        " ARRAY JOIN h.value_ids AS ref"
        " LEFT JOIN silver.class_task_field_text AS t FINAL"
        "        ON t.tenant_id = h.tenant_id"
        "       AND t.insight_source_id = h.insight_source_id"
        "       AND t.data_source = h.data_source"
        "       AND t.text_id = ref"
        " WHERE h.insight_source_id = {src:String} AND h.value_id_type = 'text_ref'"
        "   AND ifNull(t.text_id, '') = ''"
        " UNION ALL"
        " SELECT 'current' AS event_id, c.text_id AS text_id"
        " FROM silver.class_task_field_current_text AS c FINAL"
        " LEFT JOIN silver.class_task_field_text AS t FINAL"
        "        ON t.tenant_id = c.tenant_id"
        "       AND t.insight_source_id = c.insight_source_id"
        "       AND t.data_source = c.data_source"
        "       AND t.text_id = c.text_id"
        " WHERE c.insight_source_id = {src:String} AND ifNull(t.text_id, '') = ''",
        {"src": silver.source},
    )
    assert unresolved == []


def test_the_current_document_is_found_by_the_issue_id(silver: Scenario) -> None:
    """After a change the history's newest body is Jira's rendering; the issue's
    own document is reached through its immutable id, never its key."""
    rows = silver.warehouse.rows(
        "SELECT c.issue_id AS issue_id, t.content_form AS content_form, t.content AS content"
        " FROM silver.class_task_field_current_text AS c FINAL"
        " INNER JOIN silver.class_task_field_text AS t FINAL"
        "         ON t.tenant_id = c.tenant_id"
        "        AND t.insight_source_id = c.insight_source_id"
        "        AND t.data_source = c.data_source"
        "        AND t.text_id = c.text_id"
        " WHERE c.insight_source_id = {src:String} AND c.field_id = 'description'",
        {"src": silver.source},
    )
    assert [(r["issue_id"], r["content_form"]) for r in rows] == [(ISSUE_ID, "adf_json")]
    assert json.loads(rows[0]["content"]) == DESCRIPTION_ADF
