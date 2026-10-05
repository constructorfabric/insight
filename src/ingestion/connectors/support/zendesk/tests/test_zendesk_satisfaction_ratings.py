"""Mock-server tests for the `zendesk_satisfaction_ratings` stream.

Incremental CSAT over GET /satisfaction_ratings with Zendesk cursor pagination.
Zendesk caps OFFSET pagination at 10,000 records and answers 400 past it, so a
backfill wider than that used to abort the sync outright; the stream now pages
by page[size] / page[after].

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

_STREAM = "zendesk_satisfaction_ratings"
_CONNECTOR = "support/zendesk"
_URL = f"{BASE_URL}/satisfaction_ratings"


def _params(*, start_time: str = START_EPOCH, after: str | None = None) -> dict[str, Any]:
    params = {"page[size]": "100", "start_time": start_time}
    if after:
        params["page[after]"] = after
    return params


def _response(
    ratings: list[dict[str, Any]], *, after_cursor: str | None = None, boundary_indicator: bool = True
) -> HttpResponse:
    meta: dict[str, object] = {"after_cursor": after_cursor or "end"}
    if boundary_indicator:
        meta["has_more"] = after_cursor is not None
    body = {"satisfaction_ratings": ratings, "meta": meta}
    return HttpResponse(body=json.dumps(body), status_code=200)


def _rating(rating_id: int, created_at: str, **overrides: Any) -> dict[str, Any]:
    return load_fixture(
        __file__, "rating.json", id=rating_id, created_at=created_at, updated_at=created_at, **overrides
    )


@freezegun.freeze_time(NOW)
def test_full_refresh_single_page(http_mocker: HttpMocker) -> None:
    """One page of ratings, all emitted."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        _response([_rating(7001, "2026-06-16T08:00:00Z"), _rating(7002, "2026-06-17T08:00:00Z")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert sorted(r.record.data["rating_id"] for r in output.records) == ["7001", "7002"]


@freezegun.freeze_time(NOW)
def test_empty_page(http_mocker: HttpMocker) -> None:
    """No ratings in the window — no records, no error."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _response([]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert output.records == []
    assert not output.errors


@freezegun.freeze_time(NOW)
def test_schema_conformance(http_mocker: HttpMocker) -> None:
    """strict=False: the raw rating passes through beside the declared columns."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _response([_rating(7001, "2026-06-16T08:00:00Z")])
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM, strict=False)


@freezegun.freeze_time(NOW)
def test_tenant_source_stamping_and_transformations(http_mocker: HttpMocker) -> None:
    """Identity stamping and the id mappings. `reason` falls back to the
    numeric reason_code and must still land as a string."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        _response([_rating(7001, "2026-06-16T08:00:00Z", reason=None, reason_code=3)]),
    )

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["rating_id"] == "7001"
    assert rec["ticket_id"] == "1001"
    # attribution key for CSAT — the assignee, not the actor (PRD §4)
    assert rec["assignee_id"] == "3001"
    assert rec["reason"] == "3", "numeric reason_code must not land as an int"
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-7001")


@freezegun.freeze_time(NOW)
def test_pagination_multi_page(http_mocker: HttpMocker) -> None:
    """meta.has_more drives a second request carrying page[after]."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=_params()),
        _response([_rating(7001, "2026-06-16T08:00:00Z")], after_cursor="cur-2"),
    )
    http_mocker.get(
        HttpRequest(_URL, query_params=_params(after="cur-2")), _response([_rating(7002, "2026-06-17T08:00:00Z")])
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 2


@freezegun.freeze_time(NOW)
def test_pagination_survives_an_absent_has_more(http_mocker: HttpMocker) -> None:
    """Same contract as support_agents: a page carrying after_cursor but no
    has_more is not the end of the read."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=_params()),
        _response([_rating(7001, "2026-06-16T08:00:00Z")], after_cursor="cur-2", boundary_indicator=False),
    )
    http_mocker.get(
        HttpRequest(_URL, query_params=_params(after="cur-2")), _response([_rating(7002, "2026-06-17T08:00:00Z")])
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 2


@freezegun.freeze_time(NOW)
def test_error_retry_on_429(http_mocker: HttpMocker) -> None:
    """429 is retried, not fatal."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        [
            HttpResponse(body="{}", status_code=429, headers={"Retry-After": "0"}),
            _response([_rating(7001, "2026-06-16T08:00:00Z")]),
        ],
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 1


@freezegun.freeze_time(NOW)
def test_incremental_state_emitted_and_resume_filters(http_mocker: HttpMocker) -> None:
    """State at the max observed created_at; the resume read starts from the
    cursor minus the P1D lookback."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(HttpRequest(_URL, query_params=_params()), _response([_rating(7001, "2026-06-16T08:00:00Z")]))

    first = read_stream(_CONNECTOR, _STREAM, config)

    assert first.state_messages, "incremental stream must emit state"
    state = [m.state for m in first.state_messages][-1:]

    resume_start = "1781510400"  # 2026-06-15T08:00:00Z
    resume_mocker = HttpMocker()
    with resume_mocker:
        resume_mocker.get(HttpRequest(_URL, query_params=_params(start_time=resume_start)), _response([]))

        second = read_stream(_CONNECTOR, _STREAM, config, state=state)

        assert second.records == []
        assert not second.errors
