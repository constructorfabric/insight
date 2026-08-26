"""Mock-server tests for the `jira_sprints` stream.

Substream of the inline `_scrum_boards` parent (GET /rest/agile/1.0/board
?type=scrum — sprints exist only on scrum boards): one
GET /rest/agile/1.0/board/{board_id}/sprint per board partition.

Coverage matrix rows: per_board_fan_out, tenant_source_stamping.
"""

from __future__ import annotations

import json

from config import JIRA_URL, JiraConfigBuilder
from connector_tests import HttpMocker, HttpRequest, HttpResponse, read_stream

_STREAM = "jira_sprints"
_CONNECTOR = "task-tracking/jira"
_BOARDS_URL = f"{JIRA_URL}/rest/agile/1.0/board"


def _mock_scrum_boards(http_mocker: HttpMocker, board_ids: list[int]) -> None:
    http_mocker.get(
        HttpRequest(_BOARDS_URL, query_params={"type": "scrum", "maxResults": "50"}),
        HttpResponse(
            body=json.dumps(
                {"values": [{"id": bid, "name": f"Board {bid}", "type": "scrum"} for bid in board_ids], "isLast": True}
            ),
            status_code=200,
        ),
    )


def _sprints_response(sprints: list[dict[str, object]]) -> HttpResponse:
    return HttpResponse(body=json.dumps({"values": sprints, "isLast": True}), status_code=200)


def _sprint(sid: int, state: str) -> dict[str, object]:
    return {
        "id": sid,
        "name": f"Sprint {sid}",
        "state": state,
        "startDate": "2026-06-01T00:00:00.000Z",
        "endDate": "2026-06-14T00:00:00.000Z",
    }


def test_per_board_fan_out(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    _mock_scrum_boards(http_mocker, [7, 8])
    http_mocker.get(
        HttpRequest(f"{_BOARDS_URL}/7/sprint", query_params={"maxResults": "50"}),
        _sprints_response([_sprint(70, "closed")]),
    )
    http_mocker.get(
        HttpRequest(f"{_BOARDS_URL}/8/sprint", query_params={"maxResults": "50"}),
        _sprints_response([_sprint(80, "active")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert not output.errors
    assert sorted(r.record.data["sprint_id"] for r in output.records) == [70, 80]


def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    _mock_scrum_boards(http_mocker, [7])
    http_mocker.get(
        HttpRequest(f"{_BOARDS_URL}/7/sprint", query_params={"maxResults": "50"}),
        _sprints_response([_sprint(70, "active")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-70")
    assert rec["board_id"] == 7
    assert rec["sprint_name"] == "Sprint 70"
    assert rec["state"] == "active"
    assert rec["start_date"] == "2026-06-01T00:00:00.000Z"
