"""Mock-server tests for the `jira_comments` stream.

Substream of the lightweight jira_issue_keys parent: one
GET /rest/api/3/issue/{key}/comment per issue partition, `comments`
extraction, ADF body serialized to JSON.

Coverage matrix rows: parent_chain_fan_out, tenant_source_stamping
(body tojson).

The clock is frozen at 2026-07-01 00:00 UTC and jira_start_date is 2026-06-01,
so the parent gets exactly one 30-day slice.
"""

from __future__ import annotations

import json

import freezegun
from config import JIRA_URL, JiraConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, load_fixture, read_stream

_STREAM = "jira_comments"
_CONNECTOR = "task-tracking/jira"
_PROJECT_SEARCH_URL = f"{JIRA_URL}/rest/api/3/project/search"
_JQL_URL = f"{JIRA_URL}/rest/api/3/search/jql"
_NOW = "2026-07-01T00:00:00Z"

_ADF_BODY = {"type": "doc", "version": 1, "content": []}


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


def _comments_response(ids: list[str]) -> HttpResponse:
    comments = [
        {
            "id": cid,
            "author": {"accountId": "acc-1"},
            "created": "2026-06-15T10:00:00.000+0000",
            "updated": "2026-06-15T11:00:00.000+0000",
            "body": _ADF_BODY,
        }
        for cid in ids
    ]
    return HttpResponse(body=json.dumps({"comments": comments}), status_code=200)


@freezegun.freeze_time(_NOW)
def test_parent_chain_fan_out(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    _mock_parent_chain(http_mocker, ["PROJ1-1", "PROJ1-2"])
    http_mocker.get(
        HttpRequest(f"{JIRA_URL}/rest/api/3/issue/PROJ1-1/comment", query_params=ANY_QUERY_PARAMS),
        _comments_response(["701"]),
    )
    http_mocker.get(
        HttpRequest(f"{JIRA_URL}/rest/api/3/issue/PROJ1-2/comment", query_params=ANY_QUERY_PARAMS),
        _comments_response(["702"]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    # The lookback slice may re-emit a partition; bronze dedups by unique_key.
    assert not output.errors
    assert sorted({r.record.data["comment_id"] for r in output.records}) == [701, 702]


@freezegun.freeze_time(_NOW)
def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    _mock_parent_chain(http_mocker, ["PROJ1-1"])
    http_mocker.get(
        HttpRequest(f"{JIRA_URL}/rest/api/3/issue/PROJ1-1/comment", query_params=ANY_QUERY_PARAMS),
        _comments_response(["701"]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-701")
    assert rec["id_readable"] == "PROJ1-1"
    assert rec["author_account_id"] == "acc-1"
    assert rec["updated"] == "2026-06-15T11:00:00.000+0000"
    # Raw ADF body rides along as JSON; plaintext extraction is deferred to dbt.
    assert rec["body"] == _ADF_BODY
