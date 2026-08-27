"""Mock-server tests for the `jira_worklogs` stream.

Substream of the lightweight jira_issue_keys parent: one
GET /rest/api/3/issue/{key}/worklog per issue partition, `worklogs`
extraction.

Coverage matrix rows: parent_chain_fan_out, tenant_source_stamping.

The clock is frozen at 2026-07-01 00:00 UTC and jira_start_date is 2026-06-01,
so the parent gets exactly one 30-day slice.
"""

from __future__ import annotations

import json

import freezegun
from config import JIRA_URL, JiraConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, load_fixture, read_stream

_STREAM = "jira_worklogs"
_CONNECTOR = "task-tracking/jira"
_PROJECT_SEARCH_URL = f"{JIRA_URL}/rest/api/3/project/search"
_JQL_URL = f"{JIRA_URL}/rest/api/3/search/jql"
_NOW = "2026-07-01T00:00:00Z"


def _mock_parent_chain(http_mocker: HttpMocker, issue_keys: list[str]) -> None:
    projects = [load_fixture(__file__, "discovery_project.json", id="10000", key="PROJ1", name="Project PROJ1")]
    http_mocker.get(
        HttpRequest(_PROJECT_SEARCH_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps({"values": projects, "isLast": True}), status_code=200),
    )
    issues = [
        load_fixture(
            __file__, "issue.json", id=str(20000 + i), key=key, fields={"updated": "2026-06-15T10:00:00.000+0000"}
        )
        for i, key in enumerate(issue_keys)
    ]
    http_mocker.get(
        HttpRequest(_JQL_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps({"issues": issues}), status_code=200),
    )


def _worklogs_response(ids: list[str]) -> HttpResponse:
    worklogs = [
        {
            "id": wid,
            "author": {"accountId": "acc-1"},
            "started": "2026-06-15T09:00:00.000+0000",
            "updated": "2026-06-15T10:00:00.000+0000",
            "timeSpentSeconds": 3600,
            "comment": "worked",
        }
        for wid in ids
    ]
    return HttpResponse(body=json.dumps({"worklogs": worklogs}), status_code=200)


@freezegun.freeze_time(_NOW)
def test_parent_chain_fan_out(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    _mock_parent_chain(http_mocker, ["PROJ1-1", "PROJ1-2"])
    http_mocker.get(
        HttpRequest(f"{JIRA_URL}/rest/api/3/issue/PROJ1-1/worklog", query_params=ANY_QUERY_PARAMS),
        _worklogs_response(["801"]),
    )
    http_mocker.get(
        HttpRequest(f"{JIRA_URL}/rest/api/3/issue/PROJ1-2/worklog", query_params=ANY_QUERY_PARAMS),
        _worklogs_response(["802"]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    # The lookback slice may re-emit a partition; bronze dedups by unique_key.
    assert not output.errors
    assert sorted({r.record.data["worklog_id"] for r in output.records}) == [801, 802]


@freezegun.freeze_time(_NOW)
def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    _mock_parent_chain(http_mocker, ["PROJ1-1"])
    http_mocker.get(
        HttpRequest(f"{JIRA_URL}/rest/api/3/issue/PROJ1-1/worklog", query_params=ANY_QUERY_PARAMS),
        _worklogs_response(["801"]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-801")
    assert rec["id_readable"] == "PROJ1-1"
    assert rec["author_account_id"] == "acc-1"
    assert rec["started"] == "2026-06-15T09:00:00.000+0000"
    assert rec["time_spent_seconds"] == 3600
    assert rec["comment"] == "worked"
