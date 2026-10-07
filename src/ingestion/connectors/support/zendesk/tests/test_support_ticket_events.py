"""Mock-server tests for the `support_ticket_events` stream.

The Ticket Audits substream — the only actor-attributed source of support
activity. It fans out over the slim `support_ticket_ids` parent, one
GET /tickets/{id}/audits.json per ticket.

The regression this module exists for: the stream declared no incremental_sync,
so the CDK advertised full_refresh only, reconcile assigned full_refresh, and no
state survived a run — which made `incremental_dependency: true` a no-op and
re-enumerated the whole start_date window every night, one request per ticket,
against an endpoint capped at 10 requests/minute.
`test_formal_cursor_makes_the_stream_stateful` is the guard.

Coverage matrix rows: full_refresh_single_page, schema_conformance,
tenant_source_stamping, empty_page, error_retry, error_ignore,
pagination_multi_page, substream_partition, incremental_state, transformations.
"""

from __future__ import annotations

import json
from typing import Any

import freezegun
from config import BASE_URL, NOW, ZendeskConfigBuilder
from connector_tests import (
    ANY_QUERY_PARAMS,
    HttpMocker,
    HttpRequest,
    HttpResponse,
    assert_records_conform,
    load_fixture,
    read_stream,
)

_STREAM = "support_ticket_events"
_CONNECTOR = "support/zendesk"
_PARENT_URL = f"{BASE_URL}/incremental/tickets/cursor.json"
_PARENT_UPDATED_AT = "2026-06-15T10:00:00Z"
_PARENT_UPDATED_AT_EPOCH = "1781517600"  # _PARENT_UPDATED_AT in the parent cursor's datetime_format (%s)


def _audits_url(ticket_id: int) -> str:
    return f"{BASE_URL}/tickets/{ticket_id}/audits.json"


def _parent_response(ticket_ids: list[int]) -> HttpResponse:
    tickets = [load_fixture(__file__, "ticket.json", id=tid, updated_at=_PARENT_UPDATED_AT) for tid in ticket_ids]
    body = {
        "tickets": tickets,
        "after_cursor": "cur-end",
        "after_url": f"{_PARENT_URL}?cursor=cur-end",
        "end_of_stream": True,
    }
    return HttpResponse(body=json.dumps(body), status_code=200)


def _audits_response(audits: list[dict[str, Any]], *, next_page: str | None = None) -> HttpResponse:
    return HttpResponse(body=json.dumps({"audits": audits, "next_page": next_page}), status_code=200)


def _audit(audit_id: int, ticket_id: int, **overrides: Any) -> dict[str, Any]:
    return load_fixture(__file__, "audit.json", id=audit_id, ticket_id=ticket_id, **overrides)


def _mock_parent(mocker: HttpMocker, ticket_ids: list[int]) -> None:
    mocker.get(HttpRequest(_PARENT_URL, query_params=ANY_QUERY_PARAMS), _parent_response(ticket_ids))


@freezegun.freeze_time(NOW)
def test_substream_partition_one_request_per_ticket(http_mocker: HttpMocker) -> None:
    """One audits request per parent ticket. An unregistered partition request
    would fail the test — there is no network fallthrough."""
    config = ZendeskConfigBuilder().build()
    _mock_parent(http_mocker, [1001, 1002])
    http_mocker.get(
        HttpRequest(_audits_url(1001), query_params={"per_page": "100"}), _audits_response([_audit(9001, 1001)])
    )
    http_mocker.get(
        HttpRequest(_audits_url(1002), query_params={"per_page": "100"}), _audits_response([_audit(9002, 1002)])
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert sorted(r.record.data["audit_id"] for r in output.records) == ["9001", "9002"]


@freezegun.freeze_time(NOW)
def test_full_refresh_single_page(http_mocker: HttpMocker) -> None:
    """Every audit of the enumerated ticket is emitted."""
    config = ZendeskConfigBuilder().build()
    _mock_parent(http_mocker, [1001])
    http_mocker.get(
        HttpRequest(_audits_url(1001), query_params=ANY_QUERY_PARAMS),
        _audits_response([_audit(9001, 1001), _audit(9002, 1001)]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 2


@freezegun.freeze_time(NOW)
def test_empty_page(http_mocker: HttpMocker) -> None:
    """A ticket with no audits contributes no records and no error."""
    config = ZendeskConfigBuilder().build()
    _mock_parent(http_mocker, [1001])
    http_mocker.get(HttpRequest(_audits_url(1001), query_params=ANY_QUERY_PARAMS), _audits_response([]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert output.records == []
    assert not output.errors


@freezegun.freeze_time(NOW)
def test_schema_conformance(http_mocker: HttpMocker) -> None:
    """strict=False: the raw audit passes through beside the declared columns."""
    config = ZendeskConfigBuilder().build()
    _mock_parent(http_mocker, [1001])
    http_mocker.get(
        HttpRequest(_audits_url(1001), query_params=ANY_QUERY_PARAMS), _audits_response([_audit(9001, 1001)])
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM, strict=False)


@freezegun.freeze_time(NOW)
def test_tenant_source_stamping_and_transformations(http_mocker: HttpMocker) -> None:
    """Identity stamping, the actor id, and events[] serialised as JSON for the
    silver explode."""
    config = ZendeskConfigBuilder().build()
    _mock_parent(http_mocker, [1001])
    http_mocker.get(
        HttpRequest(_audits_url(1001), query_params=ANY_QUERY_PARAMS), _audits_response([_audit(9001, 1001)])
    )

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["audit_id"] == "9001"
    assert rec["ticket_id"] == "1001"
    # the ACTOR — the attribution key, never the assignee
    assert rec["author_id"] == "3001"
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-9001")
    events = json.loads(rec["events"])
    assert [e["type"] for e in events] == ["Comment", "Change"]


@freezegun.freeze_time(NOW)
def test_events_present_but_null_does_not_fail_the_stream(http_mocker: HttpMocker) -> None:
    """`events: null` must serialise to an empty array, not raise."""
    config = ZendeskConfigBuilder().build()
    _mock_parent(http_mocker, [1001])
    http_mocker.get(
        HttpRequest(_audits_url(1001), query_params=ANY_QUERY_PARAMS),
        _audits_response([_audit(9001, 1001, events=None)]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert json.loads(output.records[0].record.data["events"]) == []


@freezegun.freeze_time(NOW)
def test_pagination_multi_page(http_mocker: HttpMocker) -> None:
    """next_page drives a second request; a response without it stops."""
    config = ZendeskConfigBuilder().build()
    _mock_parent(http_mocker, [1001])
    page_2 = f"{_audits_url(1001)}?page=2"
    http_mocker.get(
        HttpRequest(_audits_url(1001), query_params={"per_page": "100"}),
        _audits_response([_audit(9001, 1001)], next_page=page_2),
    )
    # RequestPath substitutes next_page for the path; the requester still
    # appends its own params, so both ride on the follow-up.
    http_mocker.get(HttpRequest(f"{page_2}&per_page=100"), _audits_response([_audit(9002, 1001)]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 2


@freezegun.freeze_time(NOW)
def test_error_ignore_on_deleted_ticket(http_mocker: HttpMocker) -> None:
    """A deleted ticket still appears in the export but 404s on /audits. It is
    IGNOREd: one deleted ticket must not fail the fan-out for all the others."""
    config = ZendeskConfigBuilder().build()
    _mock_parent(http_mocker, [1001, 1002])
    http_mocker.get(
        HttpRequest(_audits_url(1001), query_params=ANY_QUERY_PARAMS),
        HttpResponse(body='{"error": "RecordNotFound"}', status_code=404),
    )
    http_mocker.get(
        HttpRequest(_audits_url(1002), query_params=ANY_QUERY_PARAMS), _audits_response([_audit(9002, 1002)])
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["audit_id"] for r in output.records] == ["9002"]


@freezegun.freeze_time(NOW)
def test_error_retry_on_429(http_mocker: HttpMocker) -> None:
    """429 is retried with Retry-After. This is the most rate-limit-prone
    stream: one request per ticket against a 10-per-minute cap."""
    config = ZendeskConfigBuilder().build()
    _mock_parent(http_mocker, [1001])
    http_mocker.get(
        HttpRequest(_audits_url(1001), query_params=ANY_QUERY_PARAMS),
        [
            HttpResponse(body="{}", status_code=429, headers={"Retry-After": "0"}),
            _audits_response([_audit(9001, 1001)]),
        ],
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 1


@freezegun.freeze_time(NOW)
def test_audits_older_than_the_window_are_not_filtered(http_mocker: HttpMocker) -> None:
    """The created_at cursor is formal: it injects no request option and must
    drop nothing client-side. A ticket touched today carries audits from months
    ago, and every one of them is the actor-attributed activity we are after."""
    config = ZendeskConfigBuilder().build()
    _mock_parent(http_mocker, [1001])
    http_mocker.get(
        HttpRequest(_audits_url(1001), query_params=ANY_QUERY_PARAMS),
        _audits_response([_audit(9001, 1001, created_at="2026-03-01T10:00:00Z"), _audit(9002, 1001)]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert sorted(r.record.data["audit_id"] for r in output.records) == ["9001", "9002"]


@freezegun.freeze_time(NOW)
def test_formal_cursor_makes_the_stream_stateful(http_mocker: HttpMocker) -> None:
    """The stream must emit state. Without a cursor the CDK reports
    full_refresh only, reconcile assigns full_refresh, and the parent state
    that incremental_dependency exists to carry is discarded every run."""
    config = ZendeskConfigBuilder().build()
    _mock_parent(http_mocker, [1001])
    http_mocker.get(
        HttpRequest(_audits_url(1001), query_params=ANY_QUERY_PARAMS), _audits_response([_audit(9001, 1001)])
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert output.state_messages, (
        "support_ticket_events must be stateful — a full-refresh substream "
        "throws away the parent cursor and re-enumerates the whole window"
    )


@freezegun.freeze_time(NOW)
def test_parent_state_is_persisted_for_the_next_run(http_mocker: HttpMocker) -> None:
    """incremental_dependency must carry the parent's updated_at into the
    emitted state, so the next run re-enumerates only tickets that moved."""
    config = ZendeskConfigBuilder().build()
    _mock_parent(http_mocker, [1001])
    http_mocker.get(
        HttpRequest(_audits_url(1001), query_params=ANY_QUERY_PARAMS), _audits_response([_audit(9001, 1001)])
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert output.state_messages, "read must close with a state message"
    final_state = output.state_messages[-1].state.stream.stream_state.__dict__
    assert "__ab_no_cursor_state_message" not in final_state
    assert final_state.get("parent_state") == {"support_ticket_ids": {"updated_at": _PARENT_UPDATED_AT_EPOCH}}, (
        final_state
    )
    # the child's own per-partition cursor, at the audit's created_at
    assert final_state["states"] == [
        {"partition": {"parent_slice": {}, "ticket_id": "1001"}, "cursor": {"created_at": "2026-06-15T10:00:00Z"}}
    ]
