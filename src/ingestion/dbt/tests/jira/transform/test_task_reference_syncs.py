from __future__ import annotations

import json
import uuid
from collections.abc import Iterator
from typing import Any

import pytest
import test_task_references as fixtures
from conftest import Scenario, Warehouse
from helpers import SOURCE_ID, TENANT_ID, event, item

Rows = list[dict[str, Any]]


def _references(warehouse: Warehouse) -> Rows:
    return warehouse.rows(
        "SELECT unique_key, insight_source_id, issue_id, id_readable, target_id, target_key,"
        " found_in, found_ref, found_kind, is_current, toString(first_seen_at) AS first_seen_at"
        " FROM silver.class_task_references FINAL ORDER BY unique_key"
    )


def _build(scenario: Scenario, *, full_refresh: bool) -> Rows:
    scenario.build(fixtures.SELECTOR, full_refresh=full_refresh)
    scenario.warehouse.dbt("run", "--select", "class_task_references")
    return _references(scenario.warehouse)


def _snapshot(*, at: str, summary: str, linked: bool) -> dict[str, Any]:
    snapshot = dict(fixtures.ISSUES[0])
    fields = json.loads(snapshot["custom_fields_json"])
    fields["summary"] = summary
    if not linked:
        fields["issuelinks"] = []
    snapshot.update(
        custom_fields_json=json.dumps(fields),
        _airbyte_raw_id=str(uuid.uuid4()),
        _airbyte_extracted_at=at,
        updated=at,
        collected_at=at,
    )
    return snapshot


@pytest.fixture(scope="module")
def syncs(warehouse: Warehouse) -> Iterator[dict[str, Rows]]:
    fixtures._clear(warehouse)
    scenario = Scenario(warehouse)
    scenario.seed(
        fields=fixtures.FIELDS,
        issues=fixtures.ISSUES,
        events=[
            *fixtures.EVENTS,
            event(
                "APP-1",
                103,
                "2026-01-07T10:00:00",
                [item("issuelinks", to="DEF-8", to_str="This issue blocks DEF-8")],
                jira_id=fixtures.POINTING_ID,
            ),
        ],
    )
    warehouse.insert("bronze_jira.jira_comments", fixtures.COMMENTS)
    warehouse.insert("bronze_jira.jira_projects", fixtures.PROJECTS)
    phases = {"initial": _build(scenario, full_refresh=True)}
    phases["unchanged"] = _build(scenario, full_refresh=False)

    warehouse.insert(
        "bronze_jira.jira_comments",
        [fixtures._comment(fixtures.LIVE_COMMENT, fixtures._doc(fixtures._text("DEF-2")), "2026-04-02T03:00:00")],
    )
    phases["comment_edited"] = _build(scenario, full_refresh=False)

    missing_parent = fixtures._comment("9004", fixtures._doc(fixtures._text("DEF-3")), "2026-05-02T03:00:00")
    missing_parent["jira_id"] = None
    warehouse.insert(
        "bronze_jira.jira_comments",
        [
            fixtures._comment("9003", fixtures._doc(fixtures._text("No reference")), "2026-05-02T03:00:00"),
            missing_parent,
        ],
    )
    phases["comment_deleted"] = _build(scenario, full_refresh=False)

    warehouse.insert("bronze_jira.jira_issue", [_snapshot(at="2026-05-03T03:00:00", summary="DEF-2", linked=True)])
    warehouse.insert(
        "bronze_jira.jira_issue_history",
        [
            event(
                "APP-1",
                102,
                "2026-05-03T02:00:00",
                [item("summary", frm_str="Follow-up of DEF-6", to_str="DEF-2")],
                extracted_at="2026-05-03T03:00:00",
                jira_id=fixtures.POINTING_ID,
            )
        ],
    )
    phases["field_edited"] = _build(scenario, full_refresh=False)

    warehouse.insert("bronze_jira.jira_issue", [_snapshot(at="2026-05-04T03:00:00", summary="DEF-2", linked=False)])
    phases["link_removed"] = _build(scenario, full_refresh=False)

    warehouse.insert("bronze_jira.jira_issue", [fixtures._def(2, key="NEW-2", extracted_at="2026-05-05T03:00:00")])
    phases["target_moved"] = _build(scenario, full_refresh=False)
    yield phases
    fixtures._clear(warehouse)


def test_an_unchanged_sync_preserves_the_exact_reference_set(syncs: dict[str, Rows]) -> None:
    assert syncs["initial"]
    assert syncs["unchanged"] == syncs["initial"]


@pytest.mark.parametrize(
    "phase",
    ["initial", "unchanged", "comment_edited", "comment_deleted", "field_edited", "link_removed", "target_moved"],
)
def test_every_sync_keeps_one_row_per_reference(syncs: dict[str, Rows], phase: str) -> None:
    keys = [row["unique_key"] for row in syncs[phase]]
    assert len(keys) == len(set(keys))


def test_an_edited_comment_replaces_its_previous_reference(syncs: dict[str, Rows]) -> None:
    rows = [row for row in syncs["comment_edited"] if row["found_in"] == "comment"]
    assert [(row["found_ref"], row["target_key"], row["target_id"]) for row in rows] == [
        (fixtures.LIVE_COMMENT, "DEF-2", "2002")
    ]


def test_a_deleted_comment_leaves_no_silver_reference(syncs: dict[str, Rows]) -> None:
    assert not [row for row in syncs["comment_deleted"] if row["found_in"] == "comment"]


def test_a_comment_without_an_issue_id_creates_no_reference(syncs: dict[str, Rows]) -> None:
    assert not [row for row in syncs["comment_deleted"] if row["found_ref"] == "9004"]


def test_an_edited_field_keeps_the_old_reference_as_history(syncs: dict[str, Rows]) -> None:
    rows = [row for row in syncs["field_edited"] if row["found_in"] == "field" and row["found_ref"] == "summary"]
    assert {(row["target_key"], row["is_current"]) for row in rows} == {("DEF-6", 0), ("DEF-2", 1)}


def test_a_removed_link_with_changelog_remains_historical(syncs: dict[str, Rows]) -> None:
    rows = [
        row for row in syncs["link_removed"] if row["found_in"] == "link" and row["issue_id"] == fixtures.POINTING_ID
    ]
    assert [(row["target_key"], row["is_current"]) for row in rows] == [("DEF-8", 0)]


def test_a_moved_target_keeps_its_reference_identity(syncs: dict[str, Rows]) -> None:
    before = {row["unique_key"]: row for row in syncs["link_removed"] if row["target_id"] == "2002"}
    after = {row["unique_key"]: row for row in syncs["target_moved"] if row["target_id"] == "2002"}
    assert before
    assert before.keys() == after.keys()
    assert {row["target_key"] for row in after.values()} == {"NEW-2"}


def _stamp(records: Rows, *, source: str, tenant: str) -> Rows:
    return [
        {
            **record,
            "source_id": source,
            "tenant_id": tenant,
            "unique_key": record["unique_key"].replace(SOURCE_ID, source).replace(TENANT_ID, tenant),
        }
        for record in records
    ]


def test_native_comment_ids_are_isolated_between_sources_and_tenants(warehouse: Warehouse) -> None:
    fixtures._clear(warehouse)
    sources = [
        ("references-source-a", TENANT_ID, "2001"),
        ("references-source-b", TENANT_ID, "2002"),
        ("references-source-c", "22222222-2222-2222-2222-222222222222", "2003"),
    ]
    for source, tenant, target_id in sources:
        warehouse.insert("bronze_jira.jira_fields", _stamp(fixtures.FIELDS, source=source, tenant=tenant))
        warehouse.insert("bronze_jira.jira_issue", _stamp(fixtures.ISSUES, source=source, tenant=tenant))
        warehouse.insert("bronze_jira.jira_issue_history", _stamp(fixtures.EVENTS, source=source, tenant=tenant))
        warehouse.insert("bronze_jira.jira_projects", _stamp(fixtures.PROJECTS, source=source, tenant=tenant))
        key = f"DEF-{int(target_id) - 2000}"
        older = fixtures._comment("9001", fixtures._doc(fixtures._text("DEF-9")), "2026-03-01T03:00:00")
        newer = fixtures._comment("9001", fixtures._doc(fixtures._text(key)), "2026-04-01T03:00:00")
        warehouse.insert("bronze_jira.jira_comments", _stamp([older, newer], source=source, tenant=tenant))

    rows = _build(Scenario(warehouse), full_refresh=True)
    comments = [row for row in rows if row["found_in"] == "comment"]
    assert {(row["insight_source_id"], row["issue_id"], row["target_id"]) for row in comments} == {
        (source, fixtures.POINTING_ID, target_id) for source, _, target_id in sources
    }
    assert len(comments) == len(sources)
    fixtures._clear(warehouse)


def test_issue_keys_beyond_the_field_preview_are_scanned(warehouse: Warehouse) -> None:
    fixtures._clear(warehouse)
    snapshot = dict(fixtures.ISSUES[0])
    fields = json.loads(snapshot["custom_fields_json"])
    fields["description"] = fixtures._doc(fixtures._text("x " * 500 + "DEF-2"))
    snapshot["custom_fields_json"] = json.dumps(fields)
    scenario = Scenario(warehouse)
    scenario.seed(fields=fixtures.FIELDS, issues=[snapshot, *fixtures.ISSUES[1:]])
    warehouse.insert("bronze_jira.jira_projects", fixtures.PROJECTS)

    rows = _build(scenario, full_refresh=True)

    description = [row for row in rows if row["found_in"] == "field" and row["found_ref"] == "description"]
    assert [(row["target_key"], row["target_id"], row["is_current"]) for row in description] == [("DEF-2", "2002", 1)]
    fixtures._clear(warehouse)


def test_an_empty_connector_build_leaves_no_references(warehouse: Warehouse) -> None:
    fixtures._clear(warehouse)
    scenario = Scenario(warehouse)

    assert _build(scenario, full_refresh=True) == []
    assert _build(scenario, full_refresh=False) == []
