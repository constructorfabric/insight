"""Mock-server tests for the `jira_worklog_deleted` stream.

Full re-read of Jira's deleted-worklog tombstone list:
GET /rest/api/3/worklog/deleted?since=0, paged via the response's `nextPage`
URL (RequestPath token swaps the request URL wholesale) until lastPage.
See specs/DELETION-AND-VISIBILITY.md.

Coverage matrix rows: full_read_single_page, pagination_next_page_url,
tenant_source_stamping, empty_list.
"""

from __future__ import annotations

import json

from config import JIRA_URL, JiraConfigBuilder
from connector_tests import HttpMocker, HttpRequest, HttpResponse, read_stream

_STREAM = "jira_worklog_deleted"
_CONNECTOR = "task-tracking/jira"
_URL = f"{JIRA_URL}/rest/api/3/worklog/deleted"


def _page(entries: list[tuple[int, int]], *, next_since: int | None = None) -> HttpResponse:
    body: dict = {
        "values": [{"worklogId": wid, "updatedTime": ts, "properties": []} for wid, ts in entries],
        "lastPage": next_since is None,
    }
    if next_since is not None:
        body["nextPage"] = f"{_URL}?since={next_since}"
    return HttpResponse(body=json.dumps(body), status_code=200)


def test_full_read_single_page(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_URL, query_params={"since": "0"}), _page([(101, 1700000000000), (102, 1700000001000)]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert not output.errors
    assert [r.record.data["worklog_id"] for r in output.records] == [101, 102]


def test_pagination_next_page_url(http_mocker: HttpMocker) -> None:
    """The paginator follows the response's nextPage URL (which carries
    since=<until>) until lastPage=true."""
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params={"since": "0"}), _page([(101, 1700000000000)], next_since=1700000000000)
    )
    http_mocker.get(HttpRequest(_URL, query_params={"since": "1700000000000"}), _page([(102, 1700000002000)]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert [r.record.data["worklog_id"] for r in output.records] == [101, 102]


def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_URL, query_params={"since": "0"}), _page([(101, 1700000000000)]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-101")
    # updatedTime on this endpoint IS the deletion timestamp.
    assert rec["deleted_at_ms"] == 1700000000000


def test_empty_list(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_URL, query_params={"since": "0"}), _page([]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 0
    assert not output.errors
