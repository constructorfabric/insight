"""The Codex leaderboard must name the page it is asking for, and the day's
headcount must survive the read.

`chatgpt_team_codex_user_daily` sorts by `credits` and walks the answer by page
number. That sort is not a total order — everyone who spent nothing ties on it
— so a person can sit past the page boundary in one answer and before it in the
next, returned by neither while both requests succeed. A declarative stream
cannot refuse its own read, so the connector's job is narrower and these tests
cover it:

* the walk must start at a page it named, not at whatever the endpoint decides
  page one is;
* the envelope's `total_users` must reach Bronze on its own stream, because it
  is the only figure that can later say a stored day was short — it is what
  the completeness gate in `chatgpt_team__ai_dev_usage` judges a read against.

The mock serves whatever page size the manifest declares, so changing that
number cannot quietly turn either test into a different one.
"""

from __future__ import annotations

import json

import yaml
from config import LEADERBOARD_URL, ChatGptTeamConfigBuilder
from connector_tests import HttpMocker, HttpRequest, HttpResponse, connector_dir, load_fixture, read_stream
from freezegun import freeze_time

_CONNECTOR = "ai/chatgpt-team"
_USER_STREAM = "chatgpt_team_codex_user_daily"
_ORG_STREAM = "chatgpt_team_codex_user_daily_org"
_DAY = "2026-08-19"
_NOW = f"{_DAY}T11:30:00Z"


def _manifest() -> dict:
    return yaml.safe_load((connector_dir(_CONNECTOR) / "connector.yaml").read_text())


def _page_size() -> int:
    stream = next(s for s in _manifest()["streams"] if s.get("name") == _USER_STREAM)
    return int(stream["retriever"]["paginator"]["pagination_strategy"]["page_size"])


def _entry(email: str) -> dict:
    handle = email.split("@")[0]
    return load_fixture(__file__, "codex_leaderboard_entry.json", user_id=f"user-{handle}", name=handle, email=email)


def _envelope(emails: list[str], roster_size: int) -> HttpResponse:
    body = load_fixture(__file__, "codex_leaderboard_page.json")
    body["data"] = [_entry(e) for e in emails]
    body["total_users"] = roster_size
    return HttpResponse(body=json.dumps(body), status_code=200)


def _params(page_size: int, page: int | None) -> dict[str, str]:
    params = {
        "client_filter": "all",
        "sort_by": "credits",
        "sort_direction": "desc",
        "start_date": _DAY,
        "end_date": _DAY,
        "page_size": str(page_size),
    }
    if page is not None:
        params["page"] = str(page)
    return params


def test_the_first_request_names_its_page(http_mocker: HttpMocker) -> None:
    """The mock answers only a request carrying `page=1`. Left implicit, the
    first request would send no page at all and the walk would continue at 2,
    so whatever the endpoint calls page one would never be read."""
    page_size = _page_size()
    roster = [f"worked-{i:02d}@example.com" for i in range(3)]
    config = ChatGptTeamConfigBuilder().with_start_date(_DAY).build()

    http_mocker.get(
        HttpRequest(LEADERBOARD_URL, query_params=_params(page_size, page=1)), _envelope(roster, len(roster))
    )

    with freeze_time(_NOW):
        output = read_stream(_CONNECTOR, _USER_STREAM, config)

    assert not output.errors, f"the read failed: {output.errors}"
    assert sorted(r.record.data["email"] for r in output.records) == sorted(roster)


def test_the_envelope_stream_keeps_the_headcount_and_drops_the_page(http_mocker: HttpMocker) -> None:
    """The org stream asks for one row and stores the envelope. `total_users`
    has to arrive; the leaderboard page must not, or the roster would be
    stored twice and every sum over it would be wrong."""
    roster = [f"worked-{i:02d}@example.com" for i in range(3)]
    config = ChatGptTeamConfigBuilder().with_start_date(_DAY).build()

    http_mocker.get(
        HttpRequest(LEADERBOARD_URL, query_params=_params(page_size=1, page=None)), _envelope(roster[:1], len(roster))
    )

    with freeze_time(_NOW):
        output = read_stream(_CONNECTOR, _ORG_STREAM, config)

    assert not output.errors, f"the read failed: {output.errors}"
    assert len(output.records) == 1, "the envelope stream should emit one row per day"

    record = output.records[0].record.data
    assert record["total_users"] == len(roster)
    assert record["date"] == _DAY
    assert "data" not in record, "the leaderboard page rode along into the envelope row"
