"""Mock-server tests for the `support_ticket_ids` stream.

The slim id-only mirror of support_tickets and the SubstreamPartitionRouter
parent for support_ticket_events. Same cursor-based incremental export, without
the metric_sets sideload — the parent records stay tiny so the CDK's
parent-record cache cannot balloon.

Coverage matrix rows: full_refresh_single_page, schema_conformance,
tenant_source_stamping, empty_page, error_retry, pagination_multi_page,
incremental_state, transformations.
"""

from __future__ import annotations

import json

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

_STREAM = "support_ticket_ids"
_CONNECTOR = "support/zendesk"
_URL = f"{BASE_URL}/incremental/tickets/cursor.json"


def _first_page_url(start_time: str = START_EPOCH) -> str:
    return f"{_URL}?start_time={start_time}&per_page=1000"


def _next_page_url(cursor: str) -> str:
    return f"{_URL}?cursor={cursor}&per_page=1000"


def _response(tickets: list[dict], *, after_cursor: str = "cur-end", end: bool = True) -> HttpResponse:
    body = {
        "tickets": tickets,
        "after_cursor": after_cursor,
        "after_url": f"{_URL}?cursor={after_cursor}",
        "end_of_stream": end,
    }
    return HttpResponse(body=json.dumps(body), status_code=200)


def _ticket(ticket_id: int, updated_at: str) -> dict:
    return load_fixture(__file__, "ticket.json", id=ticket_id, updated_at=updated_at)


@freezegun.freeze_time(NOW)
def test_full_refresh_single_page(http_mocker: HttpMocker) -> None:
    """Every ticket in the export becomes one parent record."""
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
    """An export with no tickets yields no parent partitions and no error."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _response([]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert output.records == []
    assert not output.errors


@freezegun.freeze_time(NOW)
def test_schema_conformance(http_mocker: HttpMocker) -> None:
    """strict=False for the same reason as support_tickets: the raw Zendesk
    ticket rides along beside the declared id + cursor columns."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _response([_ticket(1001, "2026-06-15T10:00:00Z")])
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM, strict=False)


@freezegun.freeze_time(NOW)
def test_tenant_source_stamping_and_cursor_field(http_mocker: HttpMocker) -> None:
    """Identity stamping, plus the cursor field kept on the emitted record —
    incremental_dependency reads the parent's updated_at from there."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _response([_ticket(1001, "2026-06-15T10:00:00Z")])
    )

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["data_source"] == "insight_zendesk"
    assert rec["ticket_id"] == "1001"
    assert rec["updated_at"] == "2026-06-15T10:00:00Z"
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-1001")


@freezegun.freeze_time(NOW)
def test_pagination_multi_page(http_mocker: HttpMocker) -> None:
    """The parent must enumerate every page: a truncated parent silently
    truncates the audit fan-out with it."""
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
    """429 is retried, not fatal."""
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
    """State at the max observed updated_at; the resume read starts from the
    cursor minus the P1D lookback."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(HttpRequest(_first_page_url()), _response([_ticket(1001, "2026-06-15T10:00:00Z")]))

    first = read_stream(_CONNECTOR, _STREAM, config)

    assert first.state_messages, "incremental stream must emit state"
    state = [m.state for m in first.state_messages][-1:]

    resume_start = "1781431200"  # 2026-06-14T10:00:00Z
    resume_mocker = HttpMocker()
    with resume_mocker:
        resume_mocker.get(HttpRequest(_first_page_url(resume_start)), _response([]))

        second = read_stream(_CONNECTOR, _STREAM, config, state=state)

        assert second.records == []
        assert not second.errors
