"""Two tenants that share a source id, an issue id, a long-text field and a body
keep their own rows in the text classes.

A body is addressed by a hash of its content, so the same body under two
tenants hashes to one `text_id`; the tenant that leads every key and join is
what keeps the two apart.
"""

from __future__ import annotations

import json
from collections.abc import Iterator
from typing import Any

import pytest
from conftest import FIELD_HISTORY_SELECTOR, Scenario
from helpers import OTHER_TENANT, SOURCE_ID, TENANT_ID, as_other_tenant, event, field, issue, item

ISSUE_ID = "1001"
OLD_BODY = "Old synthetic body " * 20
SHARED_BODY = "Shared synthetic body " * 20
CLASSES = ("class_task_field_history", "class_task_field_text", "class_task_field_current_text")
DESCRIPTION = field("description", name="Description", schema_type="string")


def _document(text: str) -> dict[str, Any]:
    return {
        "type": "doc",
        "version": 1,
        "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}],
    }


MINE = _document("Synthetic document of this tenant " * 10)
THEIRS = _document("Synthetic document of the other tenant " * 10)

CLEAR_CLASSES = (
    "DELETE FROM silver.class_task_field_history WHERE insight_source_id = {src:String} SETTINGS mutations_sync = 2",
    "DELETE FROM silver.class_task_field_text WHERE insight_source_id = {src:String} SETTINGS mutations_sync = 2",
    "DELETE FROM silver.class_task_field_current_text WHERE insight_source_id = {src:String} SETTINGS mutations_sync = 2",
)


def _clear_classes(scenario: Scenario) -> None:
    for statement in CLEAR_CLASSES:
        scenario.warehouse.execute(statement, {"src": SOURCE_ID})


def _description_change() -> dict[str, Any]:
    return event(
        "TST-1",
        101,
        "2026-01-06T10:00:00",
        [item("description", frm_str=OLD_BODY, to_str=SHARED_BODY)],
        jira_id=ISSUE_ID,
    )


@pytest.fixture
def silver(scenario: Scenario) -> Iterator[Scenario]:
    _clear_classes(scenario)
    scenario.seed(
        fields=[DESCRIPTION],
        issues=[issue("TST-1", fields={"description": MINE}, jira_id=ISSUE_ID)],
        events=[_description_change()],
    )
    scenario.seed(
        fields=as_other_tenant([DESCRIPTION]),
        issues=as_other_tenant([issue("TST-1", fields={"description": THEIRS}, jira_id=ISSUE_ID)]),
        events=as_other_tenant([_description_change()]),
    )
    scenario.build(f"{FIELD_HISTORY_SELECTOR} +jira__task_field_current_text")
    scenario.warehouse.dbt("run", "--select", *CLASSES)
    yield scenario
    _clear_classes(scenario)


def test_the_same_body_is_stored_once_per_tenant(silver: Scenario) -> None:
    rows = silver.warehouse.rows(
        "SELECT tenant_id, text_id, unique_key FROM silver.class_task_field_text FINAL"
        " WHERE insight_source_id = {src:String} AND content = {body:String} ORDER BY tenant_id",
        {"src": SOURCE_ID, "body": SHARED_BODY},
    )

    assert [r["tenant_id"] for r in rows] == [TENANT_ID, OTHER_TENANT], rows
    assert len({r["text_id"] for r in rows}) == 1, f"one body should hash to one text_id: {rows}"
    assert all(r["unique_key"].startswith(f"{r['tenant_id']}-{SOURCE_ID}-jira-") for r in rows), rows


def test_each_tenants_current_body_resolves_to_its_own_document(silver: Scenario) -> None:
    rows = silver.warehouse.rows(
        "SELECT c.tenant_id AS tenant_id, t.content AS content"
        " FROM silver.class_task_field_current_text AS c FINAL"
        " INNER JOIN silver.class_task_field_text AS t FINAL"
        "         ON t.tenant_id = c.tenant_id"
        "        AND t.insight_source_id = c.insight_source_id"
        "        AND t.data_source = c.data_source"
        "        AND t.text_id = c.text_id"
        " WHERE c.insight_source_id = {src:String} AND c.issue_id = {issue:String} AND c.field_id = 'description'"
        " ORDER BY c.tenant_id",
        {"src": SOURCE_ID, "issue": ISSUE_ID},
    )

    assert [(r["tenant_id"], json.loads(r["content"])) for r in rows] == [(TENANT_ID, MINE), (OTHER_TENANT, THEIRS)]


def test_every_history_reference_resolves_within_its_own_tenant(silver: Scenario) -> None:
    """`assert_text_refs_resolve`, scoped to this source: the class tables outlive the module."""
    refs = silver.warehouse.rows(
        "SELECT h.tenant_id AS tenant_id, h.event_kind AS event_kind, t.content AS content"
        " FROM silver.class_task_field_history AS h FINAL"
        " ARRAY JOIN h.value_ids AS ref"
        " LEFT JOIN silver.class_task_field_text AS t FINAL"
        "        ON t.tenant_id = h.tenant_id"
        "       AND t.insight_source_id = h.insight_source_id"
        "       AND t.data_source = h.data_source"
        "       AND t.text_id = ref"
        " WHERE h.insight_source_id = {src:String} AND h.value_id_type = 'text_ref'"
        " ORDER BY h.tenant_id, h.event_order",
        {"src": SOURCE_ID},
    )

    assert [(r["tenant_id"], r["event_kind"], r["content"]) for r in refs] == [
        (tenant, kind, body)
        for tenant in (TENANT_ID, OTHER_TENANT)
        for kind, body in (("synthetic_initial", OLD_BODY), ("changelog", SHARED_BODY))
    ]
