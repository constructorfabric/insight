"""Mock-server tests for the `jira_issue` stream.

Full-record emitter: one JQL search per jira_project_discovery partition
(GET /rest/api/3/search/jql, fields=*all + expand=names, nextPageToken
pagination) with the same DatetimeBasedCursor window as jira_issue_keys.

Coverage matrix rows: full_record_read (field flattening + custom_fields_json),
story_points_no_default (the hardcoded field-id fallback is gone),
story_points_operator_override.

The clock is frozen at 2026-07-01 00:00 UTC and jira_start_date is 2026-06-01,
so each partition gets exactly one 30-day slice.
"""

from __future__ import annotations

import json

import freezegun
from config import JIRA_URL, JiraConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, load_fixture, read_stream

_STREAM = "jira_issue"
_CONNECTOR = "task-tracking/jira"
_PROJECT_SEARCH_URL = f"{JIRA_URL}/rest/api/3/project/search"
_JQL_URL = f"{JIRA_URL}/rest/api/3/search/jql"
_NOW = "2026-07-01T00:00:00Z"


def _projects_response(keys: list[str]) -> HttpResponse:
    values = [
        load_fixture(__file__, "discovery_project.json", id=str(10000 + i), key=key, name=f"Project {key}")
        for i, key in enumerate(keys)
    ]
    return HttpResponse(body=json.dumps({"values": values, "isLast": True}), status_code=200)


def _issue_fields(**extra: object) -> dict[str, object]:
    fields = {
        "updated": "2026-06-15T10:00:00.000+0000",
        "created": "2026-06-01T09:00:00.000+0000",
        "project": {"key": "PROJ1"},
        "status": {"id": "3", "name": "In Progress"},
        "issuetype": {"id": "10001", "name": "Story"},
        "assignee": {"accountId": "acc-1"},
        "reporter": {"accountId": "acc-2"},
        "labels": ["backend", "urgent"],
    }
    fields.update(extra)
    return fields


def _issues_response(issues: list[dict[str, object]]) -> HttpResponse:
    return HttpResponse(body=json.dumps({"issues": issues}), status_code=200)


@freezegun.freeze_time(_NOW)
def test_full_record_read(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECT_SEARCH_URL, query_params=ANY_QUERY_PARAMS), _projects_response(["PROJ1"]))
    http_mocker.get(
        HttpRequest(_JQL_URL, query_params=ANY_QUERY_PARAMS),
        _issues_response([{"id": "10001", "key": "PROJ1-1", "fields": _issue_fields()}]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    assert not output.errors
    rec = output.records[0].record.data
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-PROJ1-1")
    assert rec["jira_id"] == 10001
    assert rec["id_readable"] == "PROJ1-1"
    assert rec["project_key"] == "PROJ1"
    assert rec["status_id"] == 3
    assert rec["assignee_id"] == "acc-1"
    assert rec["labels_csv"] == "backend,urgent"
    # The full fields payload rides along for dbt extraction.
    # tojson stamps literal-eval back into a dict; the destination re-serializes
    # per the declared string schema.
    assert rec["custom_fields_json"]["status"]["name"] == "In Progress"


@freezegun.freeze_time(_NOW)
def test_story_points_no_hardcoded_default(http_mocker: HttpMocker) -> None:
    """Without an operator override the connector must NOT guess a field id:
    a value sitting in some instance's customfield must stay out of
    story_points (dbt resolves it from /field metadata instead)."""
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECT_SEARCH_URL, query_params=ANY_QUERY_PARAMS), _projects_response(["PROJ1"]))
    http_mocker.get(
        HttpRequest(_JQL_URL, query_params=ANY_QUERY_PARAMS),
        _issues_response([{"id": "10001", "key": "PROJ1-1", "fields": _issue_fields(customfield_10016=5)}]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert not rec.get("story_points")
    assert rec["custom_fields_json"]["customfield_10016"] == 5


@freezegun.freeze_time(_NOW)
def test_story_points_operator_override(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    config["jira_story_points_field_id"] = "customfield_777"
    http_mocker.get(HttpRequest(_PROJECT_SEARCH_URL, query_params=ANY_QUERY_PARAMS), _projects_response(["PROJ1"]))
    http_mocker.get(
        HttpRequest(_JQL_URL, query_params=ANY_QUERY_PARAMS),
        _issues_response([{"id": "10001", "key": "PROJ1-1", "fields": _issue_fields(customfield_777=8)}]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["story_points"] == 8
