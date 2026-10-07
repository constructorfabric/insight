"""The issue-key query that feeds the issue substreams clears the timezone offset.

jira_issue_history, jira_comments and jira_worklogs partition over the
jira_issue_keys parent. A JQL datetime literal is read in the instance's zone
while the bound is rendered from UTC, so the parent's ceiling must be the clock
plus PT14H here too — not only when jira_issue_keys is read as a stream of its
own. A ceiling at the bare clock leaves an issue changed within the instance's
offset out of the partitions, and its new worklogs, comments and history entries
wait for a later sync.

The clock is frozen at 2026-06-30 00:00 UTC and jira_start_date is 2026-06-01,
so the parent gets one slice, and the exact request matcher fails on any other
ceiling.
"""

from __future__ import annotations

import json
from datetime import UTC, datetime, timedelta
from importlib.metadata import version

import freezegun
import pytest
from config import JIRA_URL, JiraConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, load_fixture, read_stream
from connector_tests.source import load_manifest

_CONNECTOR = "task-tracking/jira"
_PROJECT_SEARCH_URL = f"{JIRA_URL}/rest/api/3/project/search"
_JQL_URL = f"{JIRA_URL}/rest/api/3/search/jql"
_NOW = "2026-06-30T00:00:00Z"
_WINDOW_START = "2026-06-01 00:00"
_UPDATED = "2026-06-29T23:50:00.000+0000"

_SUBSTREAMS = {
    "jira_worklogs": (
        "worklog",
        {
            "worklogs": [
                {
                    "id": "801",
                    "author": {"accountId": "acc-1"},
                    "started": _UPDATED,
                    "updated": _UPDATED,
                    "timeSpentSeconds": 600,
                }
            ],
            "total": 1,
            "startAt": 0,
            "maxResults": 100,
        },
    ),
    "jira_comments": (
        "comment",
        {
            "comments": [
                {"id": "901", "author": {"accountId": "acc-1"}, "created": _UPDATED, "updated": _UPDATED, "body": "x"}
            ],
            "total": 1,
            "startAt": 0,
            "maxResults": 100,
        },
    ),
    "jira_issue_history": (
        "changelog",
        {
            "values": [
                {
                    "id": "1001",
                    "author": {"accountId": "acc-1"},
                    "created": _UPDATED,
                    "items": [
                        {
                            "field": "status",
                            "fieldId": "status",
                            "from": "1",
                            "fromString": "Open",
                            "to": "3",
                            "toString": "In Progress",
                        }
                    ],
                }
            ],
            "total": 1,
            "startAt": 0,
            "maxResults": 100,
            "isLast": True,
        },
    ),
}


@pytest.mark.parametrize("stream", sorted(_SUBSTREAMS))
@freezegun.freeze_time(_NOW)
def test_parent_issue_key_query_clears_the_largest_timezone_offset(http_mocker: HttpMocker, stream: str) -> None:
    clock = datetime.strptime(_NOW, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=UTC)
    ceiling = (clock + timedelta(hours=14)).strftime("%Y-%m-%d %H:%M")
    resource, body = _SUBSTREAMS[stream]

    project = load_fixture(__file__, "discovery_project.json", id="10000", key="PROJ1", name="Project PROJ1")
    http_mocker.get(
        HttpRequest(_PROJECT_SEARCH_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps({"values": [project], "isLast": True}), status_code=200),
    )
    http_mocker.get(
        HttpRequest(
            _JQL_URL,
            query_params={
                "jql": (
                    f'project = "PROJ1" AND updated >= "{_WINDOW_START}" AND updated <= "{ceiling}" '
                    "ORDER BY updated ASC"
                ),
                "fields": "updated",
                "maxResults": "100",
            },
        ),
        HttpResponse(
            body=json.dumps(
                {
                    "issues": [
                        load_fixture(__file__, "issue.json", id="20000", key="PROJ1-1", fields={"updated": _UPDATED})
                    ]
                }
            ),
            status_code=200,
        ),
    )
    http_mocker.get(
        HttpRequest(f"{JIRA_URL}/rest/api/3/issue/PROJ1-1/{resource}", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps(body), status_code=200),
    )

    output = read_stream(_CONNECTOR, stream, JiraConfigBuilder().build())

    assert not output.errors
    assert output.records


def test_manifest_declares_the_runtime_this_suite_runs_on() -> None:
    """The deployed runtime is chosen by the manifest's major version, and this
    suite runs one CDK line. Declaring another major would ship a runtime whose
    parent ceiling none of the tests above exercised."""
    declared = load_manifest(_CONNECTOR)["version"]
    assert str(declared).split(".")[0] == version("airbyte-cdk").split(".")[0]
