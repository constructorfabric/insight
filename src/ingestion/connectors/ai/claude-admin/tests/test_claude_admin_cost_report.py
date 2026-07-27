"""Mock-server tests for the `claude_admin_cost_report` stream.

Incremental DatetimeBasedCursor (cursor_field `date`, step P1D,
cursor_granularity P0D) over GET /v1/organizations/cost_report with
bucket_width=1d and group_by[]=workspace_id&description; records extracted from
data[0].results[]; AddFields stamps tenant_id/insight_source_id/collected_at/
data_source and derives date + unique from the slice start.

The regression under test is #1901: the Admin cost API treats `ending_at` as
EXCLUSIVE and snaps `starting_at` to the UTC day start, so each slice must send
`ending_at = starting_at + P1D` (the next midnight). The old `cursor_granularity:
PT1S` sent `23:59:59Z`, which selected zero buckets and 400'd. Every request
below is registered with an EXACT `ending_at` matcher — if the connector emitted
`23:59:59Z` the request would match no fixture and the test would fail with the
unmatched request instead of touching the network.

Coverage matrix rows: full_refresh_single_page, incremental (date-windowed
slicing), tenant_source_stamping, schema_conformance, empty_page.
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

_STREAM = "claude_admin_cost_report"
_CONNECTOR = "ai/claude-admin"
_URL = f"{API_BASE}/v1/organizations/cost_report"

# Frozen "now" and start_date give three complete one-day slices:
#   2026-04-24, 2026-04-25, 2026-04-26  (each 00:00:00Z -> next 00:00:00Z)
_NOW = "2026-04-27T00:00:00Z"
_DAYS = ["2026-04-24", "2026-04-25", "2026-04-26"]


def _day_params(day: str, next_day: str) -> dict:
    """Exact query matcher for one daily slice. ending_at is the NEXT midnight
    (the #1901 fix), never 23:59:59Z."""
    return {
        "group_by[]": ["workspace_id", "description"],
        "bucket_width": "1d",
        "starting_at": f"{day}T00:00:00Z",
        "ending_at": f"{next_day}T00:00:00Z",
    }


def _bucket(day: str, next_day: str, results: list[dict]) -> HttpResponse:
    """Build a single daily-bucket cost_report response (midnight -> next midnight)."""
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
    """Next day in _DAYS, or the frozen-now midnight past the last slice."""
    idx = _DAYS.index(day)
    return _DAYS[idx + 1] if idx + 1 < len(_DAYS) else "2026-04-27"


def _register_all(mocker: HttpMocker, results_per_day: dict[str, list[dict]]) -> None:
    """Register one exact-match mock response per day in _DAYS."""
    for day in _DAYS:
        nxt = _next(day)
        mocker.get(
            HttpRequest(_URL, query_params=_day_params(day, nxt)),
            _bucket(day, nxt, results_per_day.get(day, [])),
        )


@freezegun.freeze_time(_NOW)
def test_incremental_windows_send_next_midnight_ending_at(http_mocker: HttpMocker) -> None:
    """Each daily slice sends ending_at = starting_at + P1D. If any slice sent
    23:59:59Z, its request would be unmatched and this read would fail."""
    config = ClaudeAdminConfigBuilder().build()
    _register_all(http_mocker, {day: [load_fixture(__file__, "cost_result.json")] for day in _DAYS})

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == len(_DAYS)  # one bucket per day, no gap/overlap
    dates = sorted(r.record.data["date"] for r in output.records)
    assert dates == _DAYS


@freezegun.freeze_time(_NOW)
def test_full_refresh_single_page(http_mocker: HttpMocker) -> None:
    """One day with data, the rest empty -> exactly one record for that day."""
    config = ClaudeAdminConfigBuilder().build()
    # Only the first day carries data; the rest are empty buckets.
    _register_all(http_mocker, {"2026-04-24": [load_fixture(__file__, "cost_result.json")]})

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    assert not output.errors
    assert output.records[0].record.data["date"] == "2026-04-24"


@freezegun.freeze_time(_NOW)
def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    """AddFields stamps tenant/source/data_source and derives the unique key."""
    config = ClaudeAdminConfigBuilder().build()
    _register_all(http_mocker, {"2026-04-24": [load_fixture(__file__, "cost_result.json")]})

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["insight_source_id"] == config["insight_source_id"]
    assert rec["data_source"] == "insight_claude_admin"
    # unique = date|workspace_id|description
    assert rec["unique"] == (
        "2026-04-24|wrkspc_01TESTAAAAAAAAAAAAAAAAAA|Claude Sonnet 4 Usage - Input Tokens"
    )


@freezegun.freeze_time(_NOW)
def test_schema_conformance(http_mocker: HttpMocker) -> None:
    """Emitted records validate against the declared cost_report schema."""
    config = ClaudeAdminConfigBuilder().build()
    _register_all(http_mocker, {day: [load_fixture(__file__, "cost_result.json")] for day in _DAYS})

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)


@freezegun.freeze_time(_NOW)
def test_empty_page(http_mocker: HttpMocker) -> None:
    """All-empty buckets yield zero records and no errors."""
    config = ClaudeAdminConfigBuilder().build()
    _register_all(http_mocker, {})  # every day returns an empty results list

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 0
    assert not output.errors
