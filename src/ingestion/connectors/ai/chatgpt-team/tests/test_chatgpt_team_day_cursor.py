"""The day cursor must never ask for more than one calendar day at a time.

The streams sharing `day_cursor` stamp their `date` from the request window's
start, because the vendor returns no date inside the per-user object. A window
covering two days therefore files the second day's rows under the first, and
nothing downstream can tell them apart.

An end bound pinned to a calendar midnight produces exactly that: the cursor's
final window runs from the previous midnight to the bound. These tests pin the
clock and walk the windows the CDK actually generates.
"""

from __future__ import annotations

import pytest
from freezegun import freeze_time

from connector_tests import get_source

_CONNECTOR = "ai/chatgpt-team"
_DAY_CURSOR_STREAMS = ["chatgpt_team_chat_activity", "chatgpt_team_codex_user_daily"]


def _config(start_date: str | None = None) -> dict[str, str]:
    config = {
        "insight_tenant_id": "11111111-1111-1111-1111-111111111111",
        "insight_source_id": "chatgpt-team-test",
        "chatgpt_account_id": "acc-1",
        "proxy_url": "https://proxy.invalid",
        "proxy_auth_token": "token",
    }
    if start_date:
        config["start_date"] = start_date
    return config


def _windows(stream_name: str, now: str, start_date: str | None) -> list[tuple[str, str]]:
    config = _config(start_date)
    with freeze_time(now):
        source = get_source(_CONNECTOR, config)
        stream = next(s for s in source.streams(config) if s.name == stream_name)
        return [
            (p.to_slice()["start_time"], p.to_slice()["end_time"])
            for p in stream.generate_partitions()
        ]


@pytest.mark.parametrize("stream_name", _DAY_CURSOR_STREAMS)
@pytest.mark.parametrize(
    "now",
    [
        "2026-08-19T11:30:00Z",  # mid-day, the ordinary case
        "2026-08-19T00:04:00Z",  # just after midnight
        "2026-08-19T23:56:00Z",  # just before the next one
    ],
)
@pytest.mark.parametrize(
    "start_date",
    [
        None,  # the manifest default, today - 7
        "2026-08-18",  # one day of window
        "2026-08-17",  # two days — an even span
        "2026-08-01",  # a long backfill
    ],
)
def test_every_request_window_stays_inside_one_day(stream_name, now, start_date):
    windows = _windows(stream_name, now, start_date)

    assert windows, f"{stream_name} generated no request window"
    wider = [(a, b) for a, b in windows if a != b]
    assert not wider, (
        f"{stream_name} asked for a window spanning more than one day: {wider}. "
        f"Records from it are stamped with {wider[0][0]}, so the later day's rows "
        f"would be stored under the earlier date."
    )


@pytest.mark.parametrize("stream_name", _DAY_CURSOR_STREAMS)
def test_the_window_walk_is_contiguous_and_reaches_today(stream_name):
    windows = _windows(stream_name, "2026-08-19T11:30:00Z", "2026-08-15")

    assert [a for a, _ in windows] == [
        "2026-08-15",
        "2026-08-16",
        "2026-08-17",
        "2026-08-18",
        "2026-08-19",
    ]
