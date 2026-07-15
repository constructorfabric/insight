"""Mock-server tests for the `participants` stream.

Substream of the inline `_meetings` parent (PR #1746 replaced the whole-object
`$ref: "#/streams/1"` parent with an inline definition): the parent enumerates
Dashboard meetings over the now-150d window, then one
GET /v2/metrics/meetings/{uuid}/participants per partition, with the meeting
uuid URL-escaped in the path (`/`→%2F, `+`→%2B, `=`→%3D). AddFields stamps
tenant_id / source_id / meeting_uuid (from the partition) / unique_key =
"{tenant}-{source}-{meeting_uuid}-{participant_uuid}-{join_time}".

How the sync works, for readers new to Airbyte substreams: `participants` has
no meeting list of its own — before it can fetch anything it must learn which
meetings exist. It does that through a private copy of the meetings stream
(`_meetings`, defined inline in connector.yaml) that it reads itself; the
top-level `meetings` stream from the catalog is a separate instance and shares
nothing with it. Every meeting the private parent returns becomes one
"partition" = one HTTP request to /metrics/meetings/{uuid}/participants.

Those per-meeting requests are the expensive part: they count against Zoom's
Dashboard-API "Heavy" quota (60 000 requests per day for the whole account).
If the parent is re-read over its full 150-day window every sync, one sync
costs ~25k requests on a busy account and two syncs a day exhaust the quota
(this happened on dev-vhc, 2026-07-14). The fix asserted by the last two
tests: remember between syncs how far the parent has been read (its
`end_time` cursor), and on the next sync ask the API only for meetings newer
than that. The remembering is awkward in Airbyte because state belongs to
catalog streams and the private parent is not one — its cursor can only be
saved inside the participants stream's own state, in a `parent_state` field,
and the CDK only writes that field when participants itself is incremental.
Hence the "formal" join_time cursor on participants: it filters nothing and
gates nothing; its sole purpose is to make participants emit a real state
message that `parent_state` can ride along in (`incremental_dependency: true`
on the ParentStreamConfig is what turns that on).

Coverage matrix rows: substream_partition (one child request per parent
partition, uuid escaping), transformations + tenant_source_stamping,
schema_conformance, pagination_multi_page, incremental_state (the two
parent-state tests above).
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
    """After a sync, the connector's final state message must record how far
    the private `_meetings` parent was read.

    Scenario: one sync over one meeting (ends 2026-06-15T10:30:00Z, one
    participant). The state emitted at the end must contain
    `parent_state._meetings.end_time == that end_time` — this is the value the
    next sync resumes the meeting enumeration from (see the resume test
    below), and it is exactly what was NOT saved before the fix: without a
    cursor on participants the CDK emitted only a "this stream has no cursor"
    placeholder and no parent_state, so every sync re-listed all 150 days of
    meetings. If this assert starts failing (placeholder state is back, or
    parent_state is empty), the quota fix has regressed and nightly syncs are
    ~25k requests again."""
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
    """A second sync must ask the API only for meetings newer than the saved
    cursor — not re-list the whole 150-day window.

    Sync 1 sees one meeting ending 2026-06-16T10:30:00Z and saves that as the
    parent cursor. Sync 2 is then started with sync 1's state, and three
    things are pinned:

    1. The meeting-list request must be `from=2026-06-09&to=<today>` — the
       saved cursor date minus the parent's 7-day lookback (the lookback
       re-covers meetings that were still running or landed late around the
       last sync; the resulting duplicate participant rows are removed
       downstream at read time). The mock server only answers this exact
       from/to pair, so a regression back to "always start 150 days ago"
       makes requests nothing answers and fails the test.
    2. Participant requests are made only for the meetings that narrowed
       listing returned (here: the one new meeting) — that is the whole point
       of the fix, request count follows the meeting listing.
    3. The new meeting's participant joined at 09:00 on June 10 — EARLIER
       than everything sync 1 saw. It must still be emitted: the join_time
       cursor on participants only carries state and must never be used to
       drop records (a real risk: meetings overlap in time, so "older than
       the newest thing seen so far" does not mean "already synced")."""
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
