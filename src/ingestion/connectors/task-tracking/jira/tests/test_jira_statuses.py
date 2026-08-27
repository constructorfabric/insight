"""Mock-server tests for the `jira_statuses` stream.

Root-array lookup: GET /rest/api/3/status, no pagination, statusCategory
flattened into category_id / category_name / category_key.

Coverage matrix rows: full_refresh_read, tenant_source_stamping, empty_list.
"""

from __future__ import annotations

import json

from config import JIRA_URL, JiraConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, read_stream

_STREAM = "jira_statuses"
_CONNECTOR = "task-tracking/jira"
_URL = f"{JIRA_URL}/rest/api/3/status"


def _status(sid: str, name: str, category_key: str) -> dict[str, object]:
    return {"id": sid, "name": name, "statusCategory": {"id": 3, "key": category_key, "name": category_key.title()}}


def test_full_refresh_read(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([_status("1", "Open", "new"), _status("6", "Closed", "done")]), status_code=200),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert not output.errors
    assert sorted(r.record.data["name"] for r in output.records) == ["Closed", "Open"]


def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([_status("6", "Done", "done")]), status_code=200),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-6")
    assert rec["status_id"] == 6
    # statusCategory flattening — the done-category detection downstream
    # (issue #1541) reads category_key, never the display name.
    assert rec["category_key"] == "done"
    assert rec["category_name"] == "Done"


def test_empty_list(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), HttpResponse(body="[]", status_code=200))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 0
    assert not output.errors
