"""The per-user stream and the org envelope stream must leave the same cursor.

The staging completeness gate admits a day's user rows only when the same read
stored the day's `total_users` envelope. The two streams keep separate cursors
and each re-reads its own lookback window from them, so a day the user stream
still re-reads while the envelope stream has moved past it is re-read without
its envelope, and the gate then holds that day back for good.
"""

from __future__ import annotations

import json

from config import METRICS_URL, ORG_ID, ClaudeTeamConfigBuilder
from connector_tests import HttpMocker, HttpRequest, HttpResponse, load_fixture, read_streams
from freezegun import freeze_time

_CONNECTOR = "ai/claude-team"
_USER_STREAM = "claude_team_code_metrics"
_ORG_STREAM = "claude_team_code_metrics_org"
_WORKED_DAY = "2026-08-18"
_EMPTY_DAY = "2026-08-19"
_JUST_AFTER_MIDNIGHT = f"{_EMPTY_DAY}T00:05:00Z"
_PEOPLE = ["a@example.com", "b@example.com"]


def _envelope(day: str, emails: list[str]) -> HttpResponse:
    body = load_fixture(__file__, "code_metrics_page.json")
    body["organization_id"] = ORG_ID
    body["start_date"] = day
    body["end_date"] = day
    body["total_users"] = len(emails)
    body["users"] = [
        load_fixture(__file__, "code_metrics_user.json", email=e, total_lines_accepted=10) for e in emails
    ]
    body["pagination"] = {"limit": 200, "offset": 0, "total": len(emails), "has_next": False}
    return HttpResponse(body=json.dumps(body), status_code=200)


def _mock_day(http_mocker: HttpMocker, day: str, emails: list[str]) -> None:
    base = {
        "organization_uuid": ORG_ID,
        "customer_type": "claude_ai",
        "subscription_type": "team",
        "start_date": day,
        "end_date": day,
    }
    users = base | {"sort_by": "total_lines_accepted", "sort_order": "desc", "limit": "200"}
    http_mocker.get(HttpRequest(METRICS_URL, query_params=users), _envelope(day, emails))
    http_mocker.get(HttpRequest(METRICS_URL, query_params=base | {"limit": "1"}), _envelope(day, emails))


def _final_cursor(output, stream: str) -> dict:
    states = [
        m.state.stream.stream_state
        for m in output.state_messages
        if m.state.stream.stream_descriptor.name == stream
    ]
    assert states, f"{stream} emitted no state"
    return states[-1].__dict__


def test_a_day_with_no_people_moves_neither_cursor_ahead_of_the_other(http_mocker: HttpMocker) -> None:
    """A read that crosses midnight asks for the new, still-empty day. The vendor
    answers it with an envelope and no users, and both streams must leave the same
    cursor behind, or the next read re-reads users for a day it no longer re-reads
    the envelope for."""
    _mock_day(http_mocker, _WORKED_DAY, _PEOPLE)
    _mock_day(http_mocker, _EMPTY_DAY, [])
    config = ClaudeTeamConfigBuilder().with_start_date(_WORKED_DAY).build()

    with freeze_time(_JUST_AFTER_MIDNIGHT):
        output = read_streams(_CONNECTOR, [_USER_STREAM, _ORG_STREAM], config)

    assert not output.errors, f"the read failed: {output.errors}"
    assert _final_cursor(output, _USER_STREAM) == _final_cursor(output, _ORG_STREAM)


def test_an_empty_day_stores_no_envelope(http_mocker: HttpMocker) -> None:
    """An envelope for a day with nobody on it has no user rows to vouch for, and
    storing it is what advances the envelope cursor past the user cursor."""
    _mock_day(http_mocker, _WORKED_DAY, _PEOPLE)
    _mock_day(http_mocker, _EMPTY_DAY, [])
    config = ClaudeTeamConfigBuilder().with_start_date(_WORKED_DAY).build()

    with freeze_time(_JUST_AFTER_MIDNIGHT):
        output = read_streams(_CONNECTOR, [_USER_STREAM, _ORG_STREAM], config)

    envelope_days = [r.record.data["metric_date"] for r in output.records if r.record.stream == _ORG_STREAM]
    assert not output.errors, f"the read failed: {output.errors}"
    assert envelope_days == [_WORKED_DAY]
