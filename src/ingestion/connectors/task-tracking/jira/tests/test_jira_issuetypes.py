"""Mock-server tests for the `jira_issuetypes` stream.

Root-array lookup: GET /rest/api/3/issuetype, no pagination.

Coverage matrix rows: full_refresh_read, tenant_source_stamping (subtask +
hierarchyLevel flatten).
"""

from __future__ import annotations

import json

from config import JIRA_URL, JiraConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, read_stream

_STREAM = "jira_issuetypes"
_CONNECTOR = "task-tracking/jira"
_URL = f"{JIRA_URL}/rest/api/3/issuetype"


def test_full_refresh_read(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                [
                    {"id": "10001", "name": "Story", "subtask": False, "hierarchyLevel": 0},
                    {"id": "10003", "name": "Sub-task", "subtask": True, "hierarchyLevel": -1},
                ]
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert not output.errors
    assert sorted(r.record.data["name"] for r in output.records) == ["Story", "Sub-task"]


def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps([{"id": "10003", "name": "Sub-task", "subtask": True, "hierarchyLevel": -1}]),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-10003")
    assert rec["issuetype_id"] == 10003
    assert rec["subtask"] is True
    assert rec["hierarchy_level"] == -1
