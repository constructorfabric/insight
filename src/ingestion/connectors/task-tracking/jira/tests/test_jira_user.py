"""Mock-server tests for the `jira_user` stream.

Root-array read: GET /rest/api/3/users/search, OffsetIncrement paginator with
page_size 200 (maxResults / startAt).

Coverage matrix rows: full_refresh_read, tenant_source_stamping,
pagination_offset_200.
"""

from __future__ import annotations

import json

from config import JIRA_URL, JiraConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, read_stream

_STREAM = "jira_user"
_CONNECTOR = "task-tracking/jira"
_URL = f"{JIRA_URL}/rest/api/3/users/search"


def _user(account_id: str, email: str) -> dict[str, object]:
    return {
        "accountId": account_id,
        "emailAddress": email,
        "displayName": f"User {account_id}",
        "accountType": "atlassian",
        "active": True,
    }


def test_full_refresh_read(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps([_user("acc-1", "a@example.com"), _user("acc-2", "b@example.com")]), status_code=200
        ),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert not output.errors


def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([_user("acc-1", "a@example.com")]), status_code=200),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-acc-1")
    assert rec["account_id"] == "acc-1"
    assert rec["email"] == "a@example.com"
    assert rec["account_type"] == "atlassian"
    assert rec["active"] is True


def test_pagination_offset_200(http_mocker: HttpMocker) -> None:
    """A full page (200 = page_size) triggers a second request with
    startAt=200; a short page stops pagination."""
    config = JiraConfigBuilder().build()
    page1 = [_user(f"acc-{i}", f"u{i}@example.com") for i in range(200)]
    page2 = [_user("acc-last", "last@example.com")]

    http_mocker.get(
        HttpRequest(_URL, query_params={"maxResults": "200"}), HttpResponse(body=json.dumps(page1), status_code=200)
    )
    http_mocker.get(
        HttpRequest(_URL, query_params={"maxResults": "200", "startAt": "200"}),
        HttpResponse(body=json.dumps(page2), status_code=200),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 201
    assert output.records[-1].record.data["account_id"] == "acc-last"
