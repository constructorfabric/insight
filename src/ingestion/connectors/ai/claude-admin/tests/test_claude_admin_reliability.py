"""Rate-limit / error-handler tests for the claude-admin connector (#1902).

Every stream's requester references the shared `retryable_error_handler`
(CompositeErrorHandler): RATE_LIMITED on 429 with WaitTimeFromHeader
`Retry-After`, RETRY on 5xx with exponential backoff, FAIL on 401/404. The
connector also sets `concurrency_level: 1` so streams read serially against the
org-wide Admin API limit instead of firing in parallel (the #1902 429 storm).

These tests exercise the error handler on the `claude_admin_users` stream
(simple full-refresh, `data` extractor, after_id pagination). A list of
responses on one matcher is served consecutively, so [429, 200] proves the
429 is retried and the read recovers without record loss.

Coverage matrix rows: error_retry (429 -> recover), error_retry (503 -> recover),
error_fail (401 surfaces as an error, no partial records).
"""

from __future__ import annotations

import json

from config import API_BASE, ClaudeAdminConfigBuilder

from connector_tests import (
    ANY_QUERY_PARAMS,
    HttpMocker,
    HttpRequest,
    HttpResponse,
    load_fixture,
    read_stream,
)

_STREAM = "claude_admin_users"
_CONNECTOR = "ai/claude-admin"
_URL = f"{API_BASE}/v1/organizations/users"


def _page(users: list[dict]) -> HttpResponse:
    # has_more=false stops the after_id paginator after one page.
    return HttpResponse(
        body=json.dumps({"data": users, "has_more": False, "last_id": None}),
        status_code=200,
    )


def _rate_limited() -> HttpResponse:
    # Retry-After: 0 keeps the test fast; WaitTimeFromHeader reads this header.
    return HttpResponse(
        body=json.dumps({"type": "error", "error": {"type": "rate_limit_error", "message": "Too many requests"}}),
        status_code=429,
        headers={"Retry-After": "0"},
    )


def _server_error() -> HttpResponse:
    return HttpResponse(
        body=json.dumps({"type": "error", "error": {"type": "api_error", "message": "temporary"}}),
        status_code=503,
    )


def test_error_retry_429_then_recovers(http_mocker: HttpMocker) -> None:
    """A 429 with Retry-After is retried per the handler; the read succeeds
    once the source recovers, with no record loss and no ERROR log."""
    config = ClaudeAdminConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        [_rate_limited(), _page([load_fixture(__file__, "user.json")])],
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    assert not output.errors
    assert output.records[0].record.data["email"] == "member@example.com"


def test_error_retry_503_then_recovers(http_mocker: HttpMocker) -> None:
    """Transient 5xx is retried with exponential backoff and the read recovers."""
    config = ClaudeAdminConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        [_server_error(), _page([load_fixture(__file__, "user.json")])],
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    assert not output.errors


def test_error_fail_401_surfaces_error(http_mocker: HttpMocker) -> None:
    """401 hits the FAIL branch: the read surfaces an error and emits no
    partial records (rather than silently succeeding with 0 rows)."""
    config = ClaudeAdminConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps({"type": "error", "error": {"type": "authentication_error", "message": "invalid key"}}),
            status_code=401,
        ),
    )

    output = read_stream(_CONNECTOR, _STREAM, config, expecting_exception=True)

    assert output.errors, "a 401 must surface as a stream error, not a silent 0-row success"
    assert len(output.records) == 0
