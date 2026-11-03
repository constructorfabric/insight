"""Mock-server tests for a sign-in Jira refuses.

Jira serves such a request as an anonymous one and marks it only with the
X-Seraph-LoginReason header: /project/search answers 200 with an empty list,
/users/search answers 403. Neither the connection check nor a sync may pass.
"""

from __future__ import annotations

import json
import logging

import pytest
from airbyte_cdk.models import FailureType, Status
from config import JIRA_URL, JiraConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, get_source, read_stream

_CONNECTOR = "task-tracking/jira"
_PROJECTS_URL = f"{JIRA_URL}/rest/api/3/project/search"
_USERS_URL = f"{JIRA_URL}/rest/api/3/users/search"
_REFUSED = {"X-Seraph-LoginReason": "AUTHENTICATED_FAILED"}


def _empty_projects(headers: dict[str, str] | None = None) -> HttpResponse:
    return HttpResponse(body=json.dumps({"values": [], "isLast": True}), status_code=200, headers=headers or {})


def _check_status(config: dict[str, object]) -> Status:
    source = get_source(_CONNECTOR, config)
    return source.check(logging.getLogger("test"), config).status


def test_check_fails_when_jira_marks_the_sign_in_refused(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _empty_projects(_REFUSED))

    assert _check_status(config) == Status.FAILED


def test_check_fails_when_only_the_user_listing_refuses_the_caller(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _empty_projects())
    http_mocker.get(
        HttpRequest(_USERS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps({"errorMessages": ["Forbidden"]}), status_code=403),
    )

    assert _check_status(config) == Status.FAILED


def test_check_passes_for_an_account_jira_accepts(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps({"values": [{"id": "10001", "key": "PROJ1"}], "isLast": True}), status_code=200),
    )
    http_mocker.get(
        HttpRequest(_USERS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([{"accountId": "acc-1"}]), status_code=200),
    )

    assert _check_status(config) == Status.SUCCEEDED


@pytest.mark.parametrize("stream", ["jira_projects", "jira_issue"])
def test_sync_fails_as_a_config_error_when_jira_marks_the_sign_in_refused(http_mocker: HttpMocker, stream: str) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _empty_projects(_REFUSED))

    output = read_stream(_CONNECTOR, stream, config, expecting_exception=True)

    assert not output.records, f"{stream} should emit nothing"
    failure_types = [e.trace.error.failure_type for e in output.errors]
    assert FailureType.config_error in failure_types, f"{stream} should fail as a config error: {failure_types}"
