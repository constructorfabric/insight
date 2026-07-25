"""Mock-server tests for the `claude_admin_messages_usage` stream.

Incremental DatetimeBasedCursor (cursor_field `date`, step P1D,
cursor_granularity P0D) over GET /v1/organizations/usage_report/messages with
bucket_width=1d and group_by[]=model&api_key_id&workspace_id&service_tier&
context_window; records extracted from data[0].results[].

Same #1901 regression as cost_report: `ending_at` is EXCLUSIVE, so each slice
must send `ending_at = starting_at + P1D`. This endpoint tolerates a sub-day
`ending_at` without a 400 but still returns nothing useful, so the exact
`ending_at` matcher below is the regression guard for this stream too.

Coverage matrix rows: full_refresh_single_page, incremental, tenant_source_
stamping, schema_conformance, empty_page.
"""

from __future__ import annotations

import json

import freezegun
from config import API_BASE, ClaudeAdminConfigBuilder

from connector_tests import (
    HttpMocker,
    HttpRequest,
    HttpResponse,
    assert_records_conform,
    load_fixture,
    read_stream,
)

_STREAM = "claude_admin_messages_usage"
_CONNECTOR = "ai/claude-admin"
_URL = f"{API_BASE}/v1/organizations/usage_report/messages"

_NOW = "2026-04-27T00:00:00Z"
_DAYS = ["2026-04-24", "2026-04-25", "2026-04-26"]
_GROUP_BY = ["model", "api_key_id", "workspace_id", "service_tier", "context_window"]


def _day_params(day: str, next_day: str) -> dict:
    return {
        "group_by[]": _GROUP_BY,
        "bucket_width": "1d",
        "starting_at": f"{day}T00:00:00Z",
        "ending_at": f"{next_day}T00:00:00Z",
    }


def _bucket(day: str, next_day: str, results: list[dict]) -> HttpResponse:
    body = {
        "data": [
            {
                "starting_at": f"{day}T00:00:00Z",
                "ending_at": f"{next_day}T00:00:00Z",
                "results": results,
            }
        ],
        "has_more": False,
        "next_page": None,
    }
    return HttpResponse(body=json.dumps(body), status_code=200)


def _next(day: str) -> str:
    idx = _DAYS.index(day)
    return _DAYS[idx + 1] if idx + 1 < len(_DAYS) else "2026-04-27"


def _register_all(mocker: HttpMocker, results_per_day: dict[str, list[dict]]) -> None:
    for day in _DAYS:
        nxt = _next(day)
        mocker.get(
            HttpRequest(_URL, query_params=_day_params(day, nxt)),
            _bucket(day, nxt, results_per_day.get(day, [])),
        )


@freezegun.freeze_time(_NOW)
def test_incremental_windows_send_next_midnight_ending_at(http_mocker: HttpMocker) -> None:
    config = ClaudeAdminConfigBuilder().build()
    _register_all(http_mocker, {day: [load_fixture(__file__, "messages_result.json")] for day in _DAYS})

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == len(_DAYS)
    assert sorted(r.record.data["date"] for r in output.records) == _DAYS


@freezegun.freeze_time(_NOW)
def test_full_refresh_single_page(http_mocker: HttpMocker) -> None:
    config = ClaudeAdminConfigBuilder().build()
    _register_all(http_mocker, {"2026-04-24": [load_fixture(__file__, "messages_result.json")]})

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    assert not output.errors
    assert output.records[0].record.data["date"] == "2026-04-24"


@freezegun.freeze_time(_NOW)
def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = ClaudeAdminConfigBuilder().build()
    _register_all(http_mocker, {"2026-04-24": [load_fixture(__file__, "messages_result.json")]})

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["insight_source_id"] == config["insight_source_id"]
    assert rec["data_source"] == "insight_claude_admin"
    # Derived token fields lifted from the nested cache_creation / server_tool_use.
    assert rec["cache_creation_5m_tokens"] == 500
    assert rec["cache_creation_1h_tokens"] == 1000
    assert rec["web_search_requests"] == 10
    # unique = date|model|api_key_id|workspace_id|service_tier|context_window
    assert rec["unique"] == (
        "2026-04-24|claude-sonnet-4-6|apikey_01TESTAAAAAAAAAAAAAAAAAA|"
        "wrkspc_01TESTAAAAAAAAAAAAAAAAAA|standard|0-200k"
    )


@freezegun.freeze_time(_NOW)
def test_schema_conformance(http_mocker: HttpMocker) -> None:
    config = ClaudeAdminConfigBuilder().build()
    _register_all(http_mocker, {day: [load_fixture(__file__, "messages_result.json")] for day in _DAYS})

    output = read_stream(_CONNECTOR, _STREAM, config)

    # strict=False: the stream intentionally passes through the raw API token
    # objects (`cache_creation`, `cache_read_input_tokens`, `server_tool_use`)
    # alongside the flattened columns the AddFields transforms derive from them
    # (`cache_creation_5m_tokens`, `cache_read_tokens`, `web_search_requests`,
    # …). The inline schema declares only the derived columns; production
    # tolerates the extras via `additionalProperties: true`. Declaring or
    # filtering the raw objects is a bronze-contract change tracked separately
    # from #1901 — mirrors the jira_issue_keys `fields` passthrough precedent.
    assert_records_conform(output.records, _CONNECTOR, _STREAM, strict=False)


@freezegun.freeze_time(_NOW)
def test_empty_page(http_mocker: HttpMocker) -> None:
    config = ClaudeAdminConfigBuilder().build()
    _register_all(http_mocker, {})

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 0
    assert not output.errors
