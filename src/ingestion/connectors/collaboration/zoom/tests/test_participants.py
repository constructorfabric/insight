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
schema_conformance, pagination_multi_page. incremental_state is N/A — the
stream is full-refresh; test_full_refresh_substream_emits_no_cursor_state pins
that contract (and documents why incremental_dependency is absent).
"""

from __future__ import annotations

import json

import freezegun
from config import (
    FROZEN_NOW,
    METRICS_URL,
    PARENT_MEETING_SLICES,
    ZoomConfigBuilder,
    mock_meeting_slices,
    mock_token,
)

from connector_tests import (
    HttpMocker,
    HttpRequest,
    HttpResponse,
    assert_records_conform,
    load_fixture,
    read_stream,
)

_STREAM = "participants"
_CONNECTOR = "collaboration/zoom"
_NOW = FROZEN_NOW


def _meeting(uuid: str, end_time: str) -> dict:
    start_time = end_time.replace("T10:30:00Z", "T10:00:00Z")
    return load_fixture(
        __file__, "meeting.json", uuid=uuid, start_time=start_time, end_time=end_time
    )


def _participant(puid: str, email: str, join_time: str = "2026-06-15T10:00:05Z") -> dict:
    return load_fixture(
        __file__, "participant.json", participant_uuid=puid, email=email, join_time=join_time
    )


def _meetings_page(meetings: list[dict]) -> HttpResponse:
    return HttpResponse(body=json.dumps({"meetings": meetings}), status_code=200)


def _participants_page(
    participants: list[dict], next_token: str | None = None
) -> HttpResponse:
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
    mock_meeting_slices(
        http_mocker,
        {"2026-06-01": _meetings_page(meetings)},
        slices=PARENT_MEETING_SLICES,
    )


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
        http_mocker,
        [
            _meeting("mtg/a+1==", "2026-06-15T10:30:00Z"),
            _meeting("mtg-b-2==", "2026-06-16T10:30:00Z"),
        ],
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
    assert sorted(r.record.data["participant_uuid"] for r in output.records) == [
        "part-a",
        "part-b",
    ]


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
        f"{config['insight_tenant_id']}-{config['insight_source_id']}"
        f"-mtg-uuid-1==-part-uuid-1-2026-06-15T10:00:05Z"
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
        HttpRequest(
            _participants_url("mtg-uuid-1%3D%3D"),
            query_params={**_CHILD_PARAMS, "next_page_token": "tok-2"},
        ),
        _participants_page([_participant("part-2", "bob@example.com")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2


@freezegun.freeze_time(_NOW)
def test_full_refresh_substream_emits_no_cursor_state(http_mocker: HttpMocker) -> None:
    """participants is a full-refresh substream: the CDK emits only the
    no-cursor sentinel state, and in particular NO parent_state — this is why
    `incremental_dependency` is deliberately absent from the ParentStreamConfig
    (it only piggybacks the parent cursor on an incremental child's state; on a
    full-refresh child it is a silent no-op — this test found that). Freshness
    is instead bounded by the parent's now-150d slicing, re-read each sync."""
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
    assert final_state.get("__ab_no_cursor_state_message") is True
    assert "parent_state" not in final_state
