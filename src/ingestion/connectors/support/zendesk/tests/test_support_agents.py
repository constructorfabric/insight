"""Mock-server tests for the `support_agents` stream.

Full refresh over GET /users?role[]=agent&role[]=admin with Zendesk CURSOR
pagination (page[size] / page[after], meta.after_cursor / meta.has_more).

The regression this module exists for: the manifest used to send `per_page`,
which selects Zendesk's OFFSET pagination — a response with next_page and no
`meta` object. The cursor paginator then read an empty token and stopped after
one page, truncating the roster at 100. That is not a visible gap downstream:
zendesk__support_event and zendesk__support_activity INNER JOIN this dimension,
so every agent past the first page loses all their activity silently.
`test_pagination_multi_page` is the guard.

Coverage matrix rows: full_refresh_single_page, schema_conformance,
tenant_source_stamping, empty_page, error_retry, pagination_multi_page,
transformations. incremental_state is SKIPPED — the stream declares no
incremental_sync (Zendesk exposes no reliable incremental users endpoint
across plan tiers, see the manifest comment).
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

_STREAM = "support_agents"
_CONNECTOR = "support/zendesk"
_URL = f"{BASE_URL}/users"
_ROLE_QS = "role%5B%5D=agent&role%5B%5D=admin"


def _page_url(*, after: str | None = None) -> str:
    url = f"{_URL}?{_ROLE_QS}&page%5Bsize%5D=100&include_boundary_indicators=true"
    if after:
        url += f"&page%5Bafter%5D={after}"
    return url


def _response(
    users: list[dict[str, Any]], *, after_cursor: str | None = None, boundary_indicator: bool = True
) -> HttpResponse:
    meta: dict[str, object] = {"after_cursor": after_cursor or "end"}
    if boundary_indicator:
        meta["has_more"] = after_cursor is not None
    return HttpResponse(body=json.dumps({"users": users, "meta": meta}), status_code=200)


def _agent(agent_id: int, email: str, **overrides: Any) -> dict[str, Any]:
    return load_fixture(__file__, "agent.json", id=agent_id, email=email, **overrides)


@freezegun.freeze_time(NOW)
def test_full_refresh_single_page(http_mocker: HttpMocker) -> None:
    """has_more false — every user on the page is emitted and the read stops."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        _response([_agent(3001, "sam.rivera@example.com"), _agent(3002, "lee.chan@example.com")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert sorted(r.record.data["email"] for r in output.records) == ["lee.chan@example.com", "sam.rivera@example.com"]


@freezegun.freeze_time(NOW)
def test_empty_page(http_mocker: HttpMocker) -> None:
    """An empty roster yields no records and no error."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _response([]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert output.records == []
    assert not output.errors


@freezegun.freeze_time(NOW)
def test_schema_conformance(http_mocker: HttpMocker) -> None:
    """strict=False: the raw Zendesk user passes through beside the declared
    columns (the manifest maps a subset and keeps the rest)."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _response([_agent(3001, "sam.rivera@example.com")])
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM, strict=False)


@freezegun.freeze_time(NOW)
def test_tenant_source_stamping_and_transformations(http_mocker: HttpMocker) -> None:
    """Identity stamping, the display-name/group mappings, and group_name as a
    real null rather than an empty string."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _response([_agent(3001, "sam.rivera@example.com")])
    )

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["agent_id"] == "3001"
    assert rec["display_name"] == "Sam Rivera"
    assert rec["group_id"] == "4001"
    # honest NULL until the groups lookup lands — an empty string would reach
    # dim_support_agent as a non-null blank
    assert rec.get("group_name") is None
    assert rec["is_active"] == 1


@freezegun.freeze_time(NOW)
def test_pagination_multi_page(http_mocker: HttpMocker) -> None:
    """meta.has_more drives a second request carrying page[after]. Sending
    per_page instead of page[size] returns no `meta` at all, which the cursor
    paginator reads as an empty token — one page, no error, truncated roster."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(HttpRequest(_page_url()), _response([_agent(3001, "sam.rivera@example.com")], after_cursor="cur-2"))
    http_mocker.get(HttpRequest(_page_url(after="cur-2")), _response([_agent(3002, "lee.chan@example.com")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 2


@freezegun.freeze_time(NOW)
def test_pagination_survives_an_absent_has_more(http_mocker: HttpMocker) -> None:
    """has_more is included on request on this endpoint. A page that carries
    after_cursor but no indicator must read as "more to come", not as the end:
    only an explicit has_more=false (or an empty page) stops the read."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_page_url()),
        _response([_agent(3001, "sam.rivera@example.com")], after_cursor="cur-2", boundary_indicator=False),
    )
    http_mocker.get(HttpRequest(_page_url(after="cur-2")), _response([_agent(3002, "lee.chan@example.com")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["agent_id"] for r in output.records] == ["3001", "3002"]


@freezegun.freeze_time(NOW)
def test_deactivated_agent_is_not_active(http_mocker: HttpMocker) -> None:
    """active=false (a deactivated user, should one still be listed) or
    suspended=true must not read as active; reading `suspended` alone kept the
    former active forever."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        _response(
            [
                _agent(3001, "sam.rivera@example.com", active=False, suspended=False),
                _agent(3002, "lee.chan@example.com", active=True, suspended=True),
                _agent(3003, "kim.ito@example.com", active=True, suspended=False),
            ]
        ),
    )

    by_id = {r.record.data["agent_id"]: r.record.data for r in read_stream(_CONNECTOR, _STREAM, config).records}

    assert by_id["3001"]["is_active"] == 0, "deactivated agent must not read as active"
    assert by_id["3002"]["is_active"] == 0, "suspended agent must not read as active"
    assert by_id["3003"]["is_active"] == 1


@freezegun.freeze_time(NOW)
def test_error_retry_on_429(http_mocker: HttpMocker) -> None:
    """429 is retried using Retry-After. This stream is also the `check`
    target, so a rate-limited check must not read as bad credentials."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        [
            HttpResponse(body="{}", status_code=429, headers={"Retry-After": "0"}),
            _response([_agent(3001, "sam.rivera@example.com")]),
        ],
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 1


@freezegun.freeze_time(NOW)
def test_expired_token_fails_instead_of_retrying(http_mocker: HttpMocker) -> None:
    """401 is a FAIL, not a retry: an expired token is not transient, and
    exhausting retries would hide the cause."""
    config = ZendeskConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body='{"error": "Couldn\'t authenticate you"}', status_code=401),
    )

    output = read_stream(_CONNECTOR, _STREAM, config, expecting_exception=True)

    assert output.errors
    assert output.records == []
