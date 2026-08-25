"""Mock-server tests for the `jira_board_configuration` stream.

Substream of jira_boards: GET /rest/agile/1.0/board/{board_id}/configuration,
one single-object request per board. Flattens the estimation field (the
instance's actual Story Points custom-field id), the column-to-status mapping
and the board location. See specs/DATA-COMPLETENESS.md.

Coverage matrix rows: per_board_fan_out, estimation_flattening,
tenant_source_stamping, kanban_without_estimation.
"""

from __future__ import annotations

import json

from config import JIRA_URL, JiraConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, read_stream

_STREAM = "jira_board_configuration"
_CONNECTOR = "task-tracking/jira"
_BOARDS_URL = f"{JIRA_URL}/rest/agile/1.0/board"


def _mock_boards(http_mocker: HttpMocker, board_ids: list[int]) -> None:
    http_mocker.get(
        HttpRequest(_BOARDS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                {"values": [{"id": bid, "name": f"Board {bid}", "type": "scrum"} for bid in board_ids], "isLast": True}
            ),
            status_code=200,
        ),
    )


def _config_response(board_id: int, *, estimation: bool = True) -> HttpResponse:
    body: dict = {
        "id": board_id,
        "name": f"Board {board_id}",
        "type": "scrum" if estimation else "kanban",
        "columnConfig": {
            "columns": [{"name": "To Do", "statuses": [{"id": "1"}]}, {"name": "Done", "statuses": [{"id": "6"}]}]
        },
        "location": {"type": "project", "key": "PROJ1"},
    }
    if estimation:
        body["estimation"] = {"type": "field", "field": {"fieldId": "customfield_10101", "displayName": "Story Points"}}
    return HttpResponse(body=json.dumps(body), status_code=200)


def test_per_board_fan_out(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    _mock_boards(http_mocker, [7, 8])
    http_mocker.get(HttpRequest(f"{_BOARDS_URL}/7/configuration"), _config_response(7))
    http_mocker.get(HttpRequest(f"{_BOARDS_URL}/8/configuration"), _config_response(8))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert not output.errors
    assert sorted(r.record.data["board_id"] for r in output.records) == [7, 8]


def test_estimation_flattening(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    _mock_boards(http_mocker, [7])
    http_mocker.get(HttpRequest(f"{_BOARDS_URL}/7/configuration"), _config_response(7))

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["estimation_field_id"] == "customfield_10101"
    assert rec["estimation_field_name"] == "Story Points"
    assert rec["board_type"] == "scrum"


def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    _mock_boards(http_mocker, [7])
    http_mocker.get(HttpRequest(f"{_BOARDS_URL}/7/configuration"), _config_response(7))

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-7")


def test_kanban_without_estimation(http_mocker: HttpMocker) -> None:
    """Kanban boards carry no estimation block; the flattened fields must be
    empty, not crash the slice."""
    config = JiraConfigBuilder().build()
    _mock_boards(http_mocker, [9])
    http_mocker.get(HttpRequest(f"{_BOARDS_URL}/9/configuration"), _config_response(9, estimation=False))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    rec = output.records[0].record.data
    assert not rec.get("estimation_field_id")
    assert rec["board_type"] == "kanban"
