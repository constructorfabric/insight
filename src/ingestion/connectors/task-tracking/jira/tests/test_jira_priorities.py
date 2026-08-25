"""Mock-server tests for the `jira_priorities` stream.

Root-array lookup: GET /rest/api/3/priority, no pagination.

Coverage matrix rows: full_refresh_read, tenant_source_stamping.
"""

from __future__ import annotations

import json

from config import JIRA_URL, JiraConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, read_stream

_STREAM = "jira_priorities"
_CONNECTOR = "task-tracking/jira"
_URL = f"{JIRA_URL}/rest/api/3/priority"


def test_full_refresh_read(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([{"id": "1", "name": "Highest"}, {"id": "3", "name": "Medium"}]), status_code=200),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert not output.errors
    assert sorted(r.record.data["name"] for r in output.records) == ["Highest", "Medium"]


def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([{"id": "3", "name": "Medium"}]), status_code=200),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-3")
    assert rec["priority_id"] == 3
