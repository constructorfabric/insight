from __future__ import annotations

import json

import freezegun
from config import API_URL, YouTrackConfigBuilder
from connector_tests import (
    ANY_QUERY_PARAMS,
    HttpMocker,
    HttpRequest,
    HttpResponse,
    assert_records_conform,
    load_fixture,
    read_stream,
)

_CONNECTOR = "task-tracking/youtrack"
_NOW = "2026-07-01T00:00:00Z"


def test_projects_are_paginated_stamped_and_schema_valid(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    project = load_fixture(__file__, "project.json")
    http_mocker.get(
        HttpRequest(f"{API_URL}/admin/projects", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([project]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "youtrack_projects", config)

    assert not output.errors
    record = output.records[0].record.data
    assert record["unique_key"] == "test-tenant-test-source-0-1"
    assert json.loads(record["project_json"])["shortName"] == "EX"
    assert_records_conform(output.records, _CONNECTOR, "youtrack_projects")


def test_custom_field_snapshot_preserves_cardinality_and_json(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    field = load_fixture(__file__, "custom_field.json")
    http_mocker.get(
        HttpRequest(f"{API_URL}/admin/customFieldSettings/customFields", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([field]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "youtrack_custom_fields", config)

    assert not output.errors
    record = output.records[0].record.data
    assert record["field_type_id"] == "version[*]"
    assert json.loads(record["metadata_json"])["fieldType"]["id"] == "version[*]"


@freezegun.freeze_time(_NOW)
def test_field_value_snapshot_keeps_field_bundle_and_values_together(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    project = load_fixture(__file__, "project.json")
    project_field = load_fixture(__file__, "project_field.json")
    http_mocker.get(
        HttpRequest(f"{API_URL}/admin/projects", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([project]), status_code=200),
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/admin/projects/0-1/customFields", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([project_field]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "youtrack_field_values", config)

    assert not output.errors
    record = output.records[0].record.data
    assert record["field_id"] == "58-1"
    assert record["field_type_id"] == "version[*]"
    assert record["bundle_id"] == "72-1"
    assert json.loads(record["values_json"])[0]["id"] == "133-1"
    assert_records_conform(output.records, _CONNECTOR, "youtrack_field_values")


@freezegun.freeze_time(_NOW)
def test_issue_payload_keeps_custom_fields_as_json(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    issue = load_fixture(__file__, "issue.json")
    http_mocker.get(
        HttpRequest(f"{API_URL}/issues", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([issue]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "youtrack_issues", config)

    assert not output.errors
    record = output.records[0].record.data
    assert "customFields" not in record
    assert json.loads(record["custom_fields_json"])[0]["$type"] == "MultiVersionIssueCustomField"
    assert json.loads(record["issue_json"])["idReadable"] == "EX-1"
    assert_records_conform(output.records, _CONNECTOR, "youtrack_issues")


@freezegun.freeze_time(_NOW)
def test_activity_cursor_paginates_and_preserves_polymorphic_values(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    first = load_fixture(__file__, "activity_page.json")
    second = {"afterCursor": None, "hasAfter": False, "activities": []}
    http_mocker.get(
        HttpRequest(f"{API_URL}/activitiesPage", query_params=ANY_QUERY_PARAMS),
        [HttpResponse(body=json.dumps(first), status_code=200), HttpResponse(body=json.dumps(second), status_code=200)],
    )

    output = read_stream(_CONNECTOR, "youtrack_activities", config)

    assert not output.errors
    record = output.records[0].record.data
    assert record["unique_key"] == "test-tenant-test-source-2-1.0-1"
    assert json.loads(record["field_json"])["$type"] == "CustomFilterField"
    assert json.loads(record["added_json"])[0]["name"] == "Example Version"
    assert_records_conform(output.records, _CONNECTOR, "youtrack_activities")


@freezegun.freeze_time(_NOW)
def test_issue_sprints_expand_issues_from_sprint_details(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    issue = load_fixture(__file__, "issue.json")
    sprint = load_fixture(__file__, "sprint.json")
    http_mocker.get(
        HttpRequest(f"{API_URL}/agiles", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([sprint["agile"]]), status_code=200),
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/agiles/108-1/sprints", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([sprint]), status_code=200),
    )
    sprint_issue_fields = {"id", "idReadable", "summary", "created", "updated", "resolved", "project"}
    sprint_issue = {key: value for key, value in issue.items() if key in sprint_issue_fields}
    sprint_detail = {"issues": [sprint_issue]}
    http_mocker.get(
        HttpRequest(f"{API_URL}/agiles/108-1/sprints/120-1", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps(sprint_detail), status_code=200),
    )

    output = read_stream(_CONNECTOR, "youtrack_issue_sprints", config)

    assert not output.errors
    record = output.records[0].record.data
    assert record["issue_id"] == "2-1"
    assert record["sprint_id"] == "120-1"
    assert json.loads(record["sprint_json"])["goal"] == "Synthetic sprint goal"
    assert_records_conform(output.records, _CONNECTOR, "youtrack_issue_sprints")
