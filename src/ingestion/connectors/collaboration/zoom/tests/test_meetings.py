"""Mock-server tests for the `meetings` stream.

Incremental stream over the Zoom Dashboard API: GET /v2/metrics/meetings with
type=past & page_size, DatetimeBasedCursor on `end_time` (start = now_utc() -
P150D, end = today, step P30D, cursor granularity P1D, lookback P7D) injected
as `from`/`to` date params, CursorPagination on `next_page_token`.

Coverage matrix rows: full_refresh_single_page (per-slice), schema_conformance,
tenant_source_stamping (unique_key from meeting uuid), empty_page,
pagination_multi_page, incremental_state (state emission + resume-request
filtering with the P7D lookback).

REGRESSION — dev-vhc Airbyte job 529: the Dashboard API only serves the last
six months, and the stream previously sliced from a static `zoom_start_date`
config; once that date fell out of the window Zoom answered
`400 {"code": 300, "message": "The request can only be queried for a month
that falls within the last six months."}` and every sync died at the pre-sync
connection check. The `six_month_window` test freezes the clock and pins —
with EXACT `from`/`to` matchers on every slice — that the first slice starts
at now-150d (inside the window) and no request predates it: with no network
fallthrough, a connector regressing to an out-of-window start date fails this
test with an unmatched-request error.
"""

from __future__ import annotations

import json

import freezegun
from config import FROZEN_NOW, METRICS_URL, ZoomConfigBuilder, metrics_params, mock_meeting_slices, mock_token
from connector_tests import HttpMocker, HttpRequest, HttpResponse, assert_records_conform, load_fixture, read_stream

_STREAM = "meetings"
_CONNECTOR = "collaboration/zoom"
_NOW = FROZEN_NOW


def _meeting(uuid: str, end_time: str) -> dict:
    start_time = end_time.replace("T10:30:00Z", "T10:00:00Z")
    return load_fixture(__file__, "meeting.json", uuid=uuid, start_time=start_time, end_time=end_time)


def _page(meetings: list[dict], next_token: str | None = None) -> HttpResponse:
    body: dict = {"meetings": meetings}
    if next_token:
        body["next_page_token"] = next_token
    return HttpResponse(body=json.dumps(body), status_code=200)


@freezegun.freeze_time(_NOW)
def test_six_month_window_regression(http_mocker: HttpMocker) -> None:
    """Job-529 regression: every slice the connector requests is pinned by an
    exact from/to matcher, first slice = now-150d. A regression to a static /
    out-of-window start date (e.g. the old zoom_start_date=2026-01-01) issues a
    request no matcher accepts and fails immediately."""
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    mock_meeting_slices(http_mocker, {"2026-06-01": _page([_meeting("mtg-uuid-1==", "2026-06-15T10:30:00Z")])})

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    assert not output.errors
    assert output.records[0].record.data["uuid"] == "mtg-uuid-1=="


@freezegun.freeze_time(_NOW)
def test_tenant_source_stamping_and_schema(http_mocker: HttpMocker) -> None:
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    mock_meeting_slices(http_mocker, {"2026-06-01": _page([_meeting("mtg-uuid-1==", "2026-06-15T10:30:00Z")])})

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-mtg-uuid-1==")
    assert_records_conform(output.records, _CONNECTOR, _STREAM)


@freezegun.freeze_time(_NOW)
def test_empty_window(http_mocker: HttpMocker) -> None:
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    mock_meeting_slices(http_mocker, {})

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 0
    assert not output.errors


@freezegun.freeze_time(_NOW)
def test_pagination_multi_page(http_mocker: HttpMocker) -> None:
    """CursorPagination inside one slice: a next_page_token drives a second
    request carrying the token AND the same from/to window."""
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    mock_meeting_slices(
        http_mocker, {"2026-06-01": _page([_meeting("mtg-uuid-1==", "2026-06-10T10:30:00Z")], next_token="tok-2")}
    )
    http_mocker.get(
        HttpRequest(METRICS_URL, query_params=metrics_params("2026-06-01", "2026-07-01", page_token="tok-2")),
        _page([_meeting("mtg-uuid-2==", "2026-06-12T10:30:00Z")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert sorted(r.record.data["uuid"] for r in output.records) == ["mtg-uuid-1==", "mtg-uuid-2=="]


@freezegun.freeze_time(_NOW)
def test_incremental_state_emitted_and_resume_filters(http_mocker: HttpMocker) -> None:
    """First read emits a state message with the max observed end_time; a
    second read given that state must slice from the cursor minus the P7D
    lookback — asserted by exact from/to matchers."""
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    mock_meeting_slices(http_mocker, {"2026-06-01": _page([_meeting("mtg-uuid-1==", "2026-06-15T10:30:00Z")])})

    first = read_stream(_CONNECTOR, _STREAM, config)

    assert len(first.records) == 1
    assert first.state_messages, "incremental stream must emit state"
    state = [m.state for m in first.state_messages][-1:]

    # Resume: cursor 2026-06-15 minus lookback P7D -> 2026-06-08.
    resume_mocker = HttpMocker()
    with resume_mocker:
        mock_token(resume_mocker)
        resume_mocker.get(HttpRequest(METRICS_URL, query_params=metrics_params("2026-06-08", "2026-07-01")), _page([]))

        second = read_stream(_CONNECTOR, _STREAM, config, state=state)

        assert len(second.records) == 0
        assert not second.errors
