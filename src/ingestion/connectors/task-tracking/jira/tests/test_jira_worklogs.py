"""Mock-server tests for the `jira_worklogs` stream.

Driven by Jira's own worklog change feed, independent of the issue streams:
GET /rest/api/3/worklog/updated?since=<cursor ms> lists the ids of changed
worklogs (paged via the response's `nextPage` URL), and
POST /rest/api/3/worklog/list fetches them in groups of up to 1000.
See specs/DELETION-AND-VISIBILITY.md.

Coverage matrix rows: feed_to_list_fan_out, list_batches_of_1000,
tenant_source_stamping, incremental_resume_from_feed_cursor, empty_feed.

The clock is frozen at 2026-07-01 00:00 UTC and jira_start_date is 2026-06-01.
"""

from __future__ import annotations

import json
from datetime import UTC, datetime

import freezegun
from config import JIRA_URL, JiraConfigBuilder
from connector_tests import HttpMocker, HttpRequest, HttpResponse, load_fixture, read_stream

_STREAM = "jira_worklogs"
_CONNECTOR = "task-tracking/jira"
_UPDATED_URL = f"{JIRA_URL}/rest/api/3/worklog/updated"
_LIST_URL = f"{JIRA_URL}/rest/api/3/worklog/list"
_NOW = "2026-07-01T00:00:00Z"
_START_MS = str(int(datetime(2026, 6, 1, tzinfo=UTC).timestamp() * 1000))


def _feed_page(entries: list[tuple[int, int]], *, next_since: int | None = None) -> HttpResponse:
    body: dict = {
        "values": [{"worklogId": wid, "updatedTime": ts, "properties": []} for wid, ts in entries],
        "lastPage": next_since is None,
    }
    if next_since is not None:
        body["nextPage"] = f"{_UPDATED_URL}?since={next_since}"
    return HttpResponse(body=json.dumps(body), status_code=200)


def _list_request(ids: list[int]) -> HttpRequest:
    return HttpRequest(_LIST_URL, body={"ids": ids})


def _list_response(ids: list[int]) -> HttpResponse:
    return HttpResponse(
        body=json.dumps([load_fixture(__file__, "worklog.json", id=str(wid)) for wid in ids]), status_code=200
    )


@freezegun.freeze_time(_NOW)
def test_feed_to_list_fan_out(http_mocker: HttpMocker) -> None:
    """Every id the feed lists, across its pages, is fetched through
    /worklog/list — no issue is consulted."""
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_UPDATED_URL, query_params={"since": _START_MS}),
        _feed_page([(801, 1781517600000), (802, 1781517601000)], next_since=1781517601000),
    )
    http_mocker.get(
        HttpRequest(_UPDATED_URL, query_params={"since": "1781517601000"}), _feed_page([(803, 1781517602000)])
    )
    http_mocker.post(_list_request([801, 802, 803]), _list_response([801, 802, 803]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert sorted(r.record.data["worklog_id"] for r in output.records) == [801, 802, 803]


@freezegun.freeze_time(_NOW)
def test_list_batches_of_1000(http_mocker: HttpMocker) -> None:
    """/worklog/list accepts at most 1000 ids, so 1001 changed worklogs take
    two requests."""
    config = JiraConfigBuilder().build()
    ids = list(range(1, 1002))
    http_mocker.get(
        HttpRequest(_UPDATED_URL, query_params={"since": _START_MS}),
        _feed_page([(wid, 1781517600000 + wid) for wid in ids]),
    )
    http_mocker.post(_list_request(ids[:1000]), _list_response(ids[:1000]))
    http_mocker.post(_list_request(ids[1000:]), _list_response(ids[1000:]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len({r.record.data["worklog_id"] for r in output.records}) == 1001


@freezegun.freeze_time(_NOW)
def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_UPDATED_URL, query_params={"since": _START_MS}), _feed_page([(801, 1781517600000)]))
    http_mocker.post(_list_request([801]), _list_response([801]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == f"{config['insight_tenant_id']}-{config['insight_source_id']}-801"
    # Jira's own issueId — an int after the CDK's literal-eval of the rendered value.
    assert rec["jira_id"] == 20000
    assert rec["author_account_id"] == "acc-1"
    assert rec["started"] == "2026-06-15T09:00:00.000+0000"
    assert rec["time_spent_seconds"] == 3600
    assert rec["comment"] == "worked"


@freezegun.freeze_time(_NOW)
def test_incremental_resume_from_feed_cursor(http_mocker: HttpMocker) -> None:
    """The feed's cursor travels in the stream state; a resumed read asks the
    feed only for changes after the newest one already returned."""
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_UPDATED_URL, query_params={"since": _START_MS}),
        _feed_page([(801, 1781517600000), (802, 1781517605000)]),
    )
    http_mocker.post(_list_request([801, 802]), _list_response([801, 802]))

    first = read_stream(_CONNECTOR, _STREAM, config)

    assert first.state_messages, "incremental stream must emit state"
    state = [m.state for m in first.state_messages][-1:]

    resume_mocker = HttpMocker()
    with resume_mocker:
        resume_mocker.get(
            HttpRequest(_UPDATED_URL, query_params={"since": "1781517605000"}), _feed_page([(803, 1781517609000)])
        )
        resume_mocker.post(_list_request([803]), _list_response([803]))

        second = read_stream(_CONNECTOR, _STREAM, config, state=state)

        assert not second.errors
        assert [r.record.data["worklog_id"] for r in second.records] == [803]


@freezegun.freeze_time(_NOW)
def test_empty_feed(http_mocker: HttpMocker) -> None:
    """No changed worklogs: no /worklog/list request, no records."""
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_UPDATED_URL, query_params={"since": _START_MS}), _feed_page([]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert output.records == []
