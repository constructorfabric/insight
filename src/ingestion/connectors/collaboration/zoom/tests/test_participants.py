"""Mock-server tests for the `participants` stream.

Substream of the inline `_meetings` parent (PR #1746 replaced the whole-object
`$ref: "#/streams/1"` parent with an inline definition): the parent enumerates
Dashboard meetings over the now-150d window, then one
GET /v2/metrics/meetings/{uuid}/participants per partition, with the meeting
uuid URL-escaped in the path (`/`→%2F, `+`→%2B, `=`→%3D). AddFields stamps
tenant_id / source_id / meeting_uuid (from the partition) / unique_key =
"{tenant}-{source}-{meeting_uuid}-{participant_uuid}-{join_time}".

Coverage matrix rows: substream_partition (one child request per parent
partition, uuid escaping), transformations + tenant_source_stamping,
schema_conformance, pagination_multi_page, incremental_state — the stream
carries a formal join_time cursor (no request options, no client-side
filtering) purely so that `incremental_dependency: true` can persist the
parent cursor as `parent_state`; the resume test pins that a second sync
enumerates only meetings newer than the saved parent cursor minus its P7D
lookback (the Zoom Heavy-quota fix: ~hundreds of requests per sync instead of
re-fanning out over the full 150-day window).
"""

from __future__ import annotations

import json

import freezegun
from config import (
    FROZEN_NOW,
    METRICS_URL,
    PARENT_MEETING_SLICES,
    ZoomConfigBuilder,
    metrics_params,
    mock_meeting_slices,
    mock_token,
)
from connector_tests import HttpMocker, HttpRequest, HttpResponse, assert_records_conform, load_fixture, read_stream

_STREAM = "participants"
_CONNECTOR = "collaboration/zoom"
_NOW = FROZEN_NOW


def _meeting(uuid: str, end_time: str) -> dict:
    start_time = end_time.replace("T10:30:00Z", "T10:00:00Z")
    return load_fixture(__file__, "meeting.json", uuid=uuid, start_time=start_time, end_time=end_time)


def _participant(puid: str, email: str, join_time: str = "2026-06-15T10:00:05Z") -> dict:
    return load_fixture(__file__, "participant.json", participant_uuid=puid, email=email, join_time=join_time)


def _meetings_page(meetings: list[dict]) -> HttpResponse:
    return HttpResponse(body=json.dumps({"meetings": meetings}), status_code=200)


def _participants_page(participants: list[dict], next_token: str | None = None) -> HttpResponse:
    body: dict = {"participants": participants}
    if next_token:
        body["next_page_token"] = next_token
    return HttpResponse(body=json.dumps(body), status_code=200)


def _mock_parent(http_mocker: HttpMocker, meetings: list[dict]) -> None:
    """The inline `_meetings` parent slices the same now-150d window as the
    top-level meetings stream (PARENT_MEETING_SLICES — the sync read path emits
    a 1-day tail instead of absorbing it); all parent meetings are served in
    the 2026-06-01 slice, the other slices are empty. Exact from/to matchers
    keep the job-529 window pin on the parent path too."""
    mock_meeting_slices(http_mocker, {"2026-06-01": _meetings_page(meetings)}, slices=PARENT_MEETING_SLICES)


def _participants_url(escaped_uuid: str) -> str:
    return f"{METRICS_URL}/{escaped_uuid}/participants"


_CHILD_PARAMS = {"type": "past", "page_size": "100"}


@freezegun.freeze_time(_NOW)
def test_substream_partition_per_meeting_with_uuid_escaping(http_mocker: HttpMocker) -> None:
    """One participants request per parent meeting; the `==` uuid suffix (and
    any `/`, `+`) must be percent-escaped in the URL path — an unescaped
    request would not match and fail the test."""
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    _mock_parent(
        http_mocker, [_meeting("mtg/a+1==", "2026-06-15T10:30:00Z"), _meeting("mtg-b-2==", "2026-06-16T10:30:00Z")]
    )
    http_mocker.get(
        HttpRequest(_participants_url("mtg%2Fa%2B1%3D%3D"), query_params=_CHILD_PARAMS),
        _participants_page([_participant("part-a", "alice@example.com")]),
    )
    http_mocker.get(
        HttpRequest(_participants_url("mtg-b-2%3D%3D"), query_params=_CHILD_PARAMS),
        _participants_page([_participant("part-b", "bob@example.com")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert not output.errors
    assert sorted(r.record.data["participant_uuid"] for r in output.records) == ["part-a", "part-b"]


@freezegun.freeze_time(_NOW)
def test_transformations_stamping_and_schema(http_mocker: HttpMocker) -> None:
    """meeting_uuid is stamped from the partition (the API payload does not
    carry it), and unique_key composes meeting_uuid + participant_uuid +
    join_time."""
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    _mock_parent(http_mocker, [_meeting("mtg-uuid-1==", "2026-06-15T10:30:00Z")])
    http_mocker.get(
        HttpRequest(_participants_url("mtg-uuid-1%3D%3D"), query_params=_CHILD_PARAMS),
        _participants_page([_participant("part-uuid-1", "alice@example.com")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["meeting_uuid"] == "mtg-uuid-1=="
    assert rec["unique_key"] == (
        f"{config['insight_tenant_id']}-{config['insight_source_id']}-mtg-uuid-1==-part-uuid-1-2026-06-15T10:00:05Z"
    )
    assert_records_conform(output.records, _CONNECTOR, _STREAM)


@freezegun.freeze_time(_NOW)
def test_pagination_multi_page(http_mocker: HttpMocker) -> None:
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    _mock_parent(http_mocker, [_meeting("mtg-uuid-1==", "2026-06-15T10:30:00Z")])
    http_mocker.get(
        HttpRequest(_participants_url("mtg-uuid-1%3D%3D"), query_params=_CHILD_PARAMS),
        _participants_page([_participant("part-1", "alice@example.com")], next_token="tok-2"),
    )
    http_mocker.get(
        HttpRequest(_participants_url("mtg-uuid-1%3D%3D"), query_params={**_CHILD_PARAMS, "next_page_token": "tok-2"}),
        _participants_page([_participant("part-2", "bob@example.com")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2


@freezegun.freeze_time(_NOW)
def test_parent_state_persisted_in_child_state(http_mocker: HttpMocker) -> None:
    """The child's cursor is a formality; what matters is that its state
    message carries `parent_state` with the `_meetings` cursor (the max observed
    end_time record value). A full-refresh child persists nothing — that
    no-op was pinned by this suite before the cursor existed — so this assert
    is what keeps the Heavy-quota fix from silently regressing."""
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    _mock_parent(http_mocker, [_meeting("mtg-uuid-1==", "2026-06-15T10:30:00Z")])
    http_mocker.get(
        HttpRequest(_participants_url("mtg-uuid-1%3D%3D"), query_params=_CHILD_PARAMS),
        _participants_page([_participant("part-1", "alice@example.com")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert output.state_messages, "read must close with a state message"
    final_state = output.state_messages[-1].state.stream.stream_state.__dict__
    assert "__ab_no_cursor_state_message" not in final_state
    parent_state = final_state.get("parent_state")
    assert parent_state == {"_meetings": {"end_time": "2026-06-15T10:30:00Z"}}, parent_state


@freezegun.freeze_time(_NOW)
def test_resume_enumerates_parent_from_saved_cursor(http_mocker: HttpMocker) -> None:
    """Second sync, given the first sync's state: the parent must be requested
    from the saved cursor minus its P7D lookback (exact from/to matcher — a
    regression back to the full now-150d fan-out would issue requests no
    matcher accepts), and participants are fetched only for the meetings that
    enumeration returns. The emitted record's join_time predates the first
    sync's records: nothing is filtered client-side — the child cursor gates
    no data, only carries state."""
    config = ZoomConfigBuilder().build()
    mock_token(http_mocker)
    _mock_parent(http_mocker, [_meeting("mtg-uuid-1==", "2026-06-16T10:30:00Z")])
    http_mocker.get(
        HttpRequest(_participants_url("mtg-uuid-1%3D%3D"), query_params=_CHILD_PARAMS),
        _participants_page([_participant("part-1", "alice@example.com", join_time="2026-06-16T10:00:05Z")]),
    )

    first = read_stream(_CONNECTOR, _STREAM, config)
    state = [m.state for m in first.state_messages][-1:]

    # Resume: parent cursor 2026-06-16 minus lookback P7D -> from=2026-06-09.
    resume_mocker = HttpMocker()
    with resume_mocker:
        mock_token(resume_mocker)
        resume_mocker.get(
            HttpRequest(METRICS_URL, query_params=metrics_params("2026-06-09", "2026-07-01")),
            _meetings_page([_meeting("mtg-new==", "2026-06-20T10:30:00Z")]),
        )
        resume_mocker.get(
            HttpRequest(_participants_url("mtg-new%3D%3D"), query_params=_CHILD_PARAMS),
            _participants_page([_participant("part-2", "bob@example.com", join_time="2026-06-10T09:00:00Z")]),
        )

        second = read_stream(_CONNECTOR, _STREAM, config, state=state)

        assert not second.errors
        assert [r.record.data["participant_uuid"] for r in second.records] == ["part-2"]
        final_state = second.state_messages[-1].state.stream.stream_state.__dict__
        assert final_state.get("parent_state") == {"_meetings": {"end_time": "2026-06-20T10:30:00Z"}}
