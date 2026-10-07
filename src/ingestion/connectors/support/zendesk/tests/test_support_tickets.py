"""Mock-server tests for the `support_tickets` stream.

Incremental over Zendesk's CURSOR-based incremental export
(GET /incremental/tickets/cursor.json), sideloading metric_sets. The regression
this module exists for: the manifest used to request the TIME-based export
(/incremental/tickets.json) while reading `after_cursor`, a field only the
cursor-based export returns. The token was therefore always empty, the CDK
ended pagination after one page without an error, and the cursor advanced past
every ticket beyond the first 1000 — permanently. `test_pagination_multi_page`
and `test_first_page_sends_start_time_and_later_pages_send_cursor` are the two
that would fail if that protocol mix came back.

Coverage matrix rows: full_refresh_single_page, schema_conformance,
tenant_source_stamping, empty_page, error_retry, pagination_multi_page,
incremental_state, transformations.
"""

from __future__ import annotations

import json
from typing import Any

import freezegun
from config import BASE_URL, NOW, START_EPOCH, ZendeskConfigBuilder
from connector_tests import (
    ANY_QUERY_PARAMS,
    HttpMocker,
    HttpRequest,
    HttpResponse,
    assert_records_conform,
    load_fixture,
    read_stream,
)

_STREAM = "support_tickets"
_CONNECTOR = "support/zendesk"
_URL = f"{BASE_URL}/incremental/tickets/cursor.json"


def _first_page_url(start_time: str = START_EPOCH) -> str:
    return f"{_URL}?start_time={start_time}&include=metric_sets&per_page=1000"


def _next_page_url(cursor: str) -> str:
    """Zendesk's after_url — the whole path for the follow-up request. The CDK
    still appends the requester's own params, so they appear here too."""
    return f"{_URL}?cursor={cursor}&include=metric_sets&per_page=1000"


def _response(tickets: list[dict[str, Any]], *, after_cursor: str = "cur-end", end: bool = True) -> HttpResponse:
    body = {
        "tickets": tickets,
        "after_cursor": after_cursor,
        "after_url": f"{_URL}?cursor={after_cursor}",
        "end_of_stream": end,
    }
    return HttpResponse(body=json.dumps(body), status_code=200)


def _ticket(ticket_id: int, updated_at: str, **overrides: Any) -> dict[str, Any]:
    return load_fixture(__file__, "ticket.json", id=ticket_id, updated_at=updated_at, **overrides)


@freezegun.freeze_time(NOW)
def test_full_refresh_single_page(http_mocker: HttpMocker) -> None:
    """One page, end_of_stream true — every ticket is emitted and the read stops."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        _response([_ticket(1001, "2026-06-15T10:00:00Z"), _ticket(1002, "2026-06-16T10:00:00Z")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert sorted(r.record.data["ticket_id"] for r in output.records) == ["1001", "1002"]


@freezegun.freeze_time(NOW)
def test_empty_page(http_mocker: HttpMocker) -> None:
    """An export with no tickets yields no records and no error."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _response([]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert output.records == []
    assert not output.errors


@freezegun.freeze_time(NOW)
def test_schema_conformance(http_mocker: HttpMocker) -> None:
    """Every emitted record validates against the stream schema.

    strict=False: this stream deliberately passes the raw Zendesk ticket
    through (`id`, `type`, `metric_set` and any sideload ride along beside the
    declared columns, and `metadata` keeps the whole payload as JSON), so
    undeclared source fields are the design, not manifest drift."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _response([_ticket(1001, "2026-06-15T10:00:00Z")])
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM, strict=False)


@freezegun.freeze_time(NOW)
def test_tenant_source_stamping_and_transformations(http_mocker: HttpMocker) -> None:
    """Identity stamping, the id-typing idiom, null-safe tags, and the
    metric_set-derived timing fields."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _response([_ticket(1001, "2026-06-15T10:00:00Z")])
    )

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["data_source"] == "insight_zendesk"
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-1001")
    # Ids must reach bronze as STRINGS: `| string` alone is undone by the CDK's
    # literal-eval, which is what `| tojson` in the manifest prevents.
    assert rec["ticket_id"] == "1001"
    assert rec["assignee_id"] == "3001"
    assert rec["tags"] == "billing,export"
    assert rec["ticket_type"] == "incident"
    assert rec["first_reply_time_seconds"] == 1800
    assert rec["first_reply_time_calendar_seconds"] == 2700
    # metric_set present but the resolution half is null -> honest null, not 0
    # AddFields omits a None-valued field rather than emitting an explicit
    # null; either way the column lands NULL in bronze.
    assert rec.get("full_resolution_time_seconds") is None
    assert rec.get("solved_at") is None


@freezegun.freeze_time(NOW)
def test_tags_present_but_null_does_not_fail_the_stream(http_mocker: HttpMocker) -> None:
    """`tags: null` must render an empty string, not raise. `.get('tags', [])`
    returns None for a present-but-null key and `None | join` fails the sync."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        _response([_ticket(1001, "2026-06-15T10:00:00Z", tags=None, metric_set=None)]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert output.records[0].record.data["tags"] == ""


@freezegun.freeze_time(NOW)
def test_pagination_multi_page(http_mocker: HttpMocker) -> None:
    """end_of_stream=false drives a second request carrying the after_cursor;
    end_of_stream=true stops. A cursor-shaped paginator pointed at the
    time-based export would read an empty token and stop after page 1."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_first_page_url()),
        _response([_ticket(1001, "2026-06-10T10:00:00Z")], after_cursor="cur-2", end=False),
    )
    http_mocker.get(HttpRequest(_next_page_url("cur-2")), _response([_ticket(1002, "2026-06-12T10:00:00Z")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert sorted(r.record.data["ticket_id"] for r in output.records) == ["1001", "1002"]


@freezegun.freeze_time(NOW)
def test_error_retry_on_429(http_mocker: HttpMocker) -> None:
    """429 is retried using Retry-After rather than failing the stream — the
    incremental-export endpoints are capped at 10 requests/minute."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        [
            HttpResponse(body="{}", status_code=429, headers={"Retry-After": "0"}),
            _response([_ticket(1001, "2026-06-15T10:00:00Z")]),
        ],
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 1


@freezegun.freeze_time(NOW)
def test_incremental_state_emitted_and_resume_filters(http_mocker: HttpMocker) -> None:
    """The first read emits state at the max observed updated_at; a resume read
    given that state requests from the cursor minus the P1D lookback."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(HttpRequest(_first_page_url()), _response([_ticket(1001, "2026-06-15T10:00:00Z")]))

    first = read_stream(_CONNECTOR, _STREAM, config)

    assert first.state_messages, "incremental stream must emit state"
    state = [m.state for m in first.state_messages][-1:]

    # cursor 2026-06-15T10:00:00Z minus lookback P1D -> 2026-06-14T10:00:00Z
    resume_start = "1781431200"
    resume_mocker = HttpMocker()
    with resume_mocker:
        resume_mocker.get(HttpRequest(_first_page_url(resume_start)), _response([]))

        second = read_stream(_CONNECTOR, _STREAM, config, state=state)

        assert second.records == []
        assert not second.errors
