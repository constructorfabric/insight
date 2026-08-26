"""Mock-server tests for the `jira_boards` stream.

Plain paginated stream: GET /rest/agile/1.0/board, `values` extraction, linked
OffsetIncrement paginator (page_size 50). Also the substream parent for
jira_board_configuration.

Coverage matrix rows: full_refresh_read, tenant_source_stamping,
pagination_offset_50.
"""

from __future__ import annotations

import json

from config import JIRA_URL, JiraConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, read_stream

_STREAM = "jira_boards"
_CONNECTOR = "task-tracking/jira"
_URL = f"{JIRA_URL}/rest/agile/1.0/board"


def _board(bid: int) -> dict[str, object]:
    return {"id": bid, "name": f"Board {bid}", "type": "scrum"}


def _page(boards: list[dict[str, object]], *, is_last: bool = True) -> HttpResponse:
    return HttpResponse(body=json.dumps({"values": boards, "isLast": is_last}), status_code=200)


def test_full_refresh_read(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _page([_board(1), _board(2)]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert not output.errors


def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _page([_board(7)]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-7")


def test_reads_the_next_page_after_a_non_final_response(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    page1 = [_board(i) for i in range(50)]
    page2 = [_board(100)]

    http_mocker.get(HttpRequest(_URL, query_params={"maxResults": "50"}), _page(page1, is_last=False))
    http_mocker.get(HttpRequest(_URL, query_params={"maxResults": "50", "startAt": "50"}), _page(page2))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 51
    assert output.records[-1].record.data["id"] == 100
