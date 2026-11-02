"""Request attribution rules exercised with external Playwright event objects.

These tests drive the observer's callbacks without opening a browser or making
requests. They verify that a late result from an earlier hop cannot fail the
page under test and that the documented failure exceptions stay narrow.
"""

from unittest.mock import Mock

import pytest
from playwright.sync_api import BrowserContext, Request, Response, Route

from .page_watch import PageWatch

pytestmark = pytest.mark.reliability


def test_late_responses_and_drops_from_an_earlier_hop_are_ignored() -> None:
    watch = PageWatch(Mock(spec=BrowserContext), "https://example.com")
    previous = Mock(
        spec=Request,
        url="https://example.com/api/previous?secret=example",
        method="GET",
        resource_type="fetch",
        failure="net::ERR_FAILED",
    )
    current = Mock(
        spec=Request,
        url="https://example.com/api/current?secret=example",
        method="GET",
        resource_type="fetch",
    )
    watch._request(previous)
    watch.mark()
    watch._request(current)
    watch._response(Mock(spec=Response, request=previous, status=500))
    watch._failed(previous)

    assert watch.failures == []
    assert watch.pending_paths() == ["/api/current"]
    watch._response(Mock(spec=Response, request=current, status=503))
    assert watch.failures == ["server_error: GET /api/current 503"]
    watch._finished(current)
    assert watch.pending_paths() == []


@pytest.mark.parametrize(
    ("failure", "status", "expected"),
    [
        ("net::ERR_ABORTED", None, []),
        ("net::ERR_FAILED", None, ["no_response: GET /api/example net::ERR_FAILED"]),
        ("net::ERR_FAILED", 200, []),
        ("net::ERR_FAILED", 404, []),
        ("net::ERR_FAILED", 500, ["server_error: GET /api/example 500"]),
    ],
)
def test_only_unanswered_non_aborted_requests_count_as_dropped(
    failure: str, status: int | None, expected: list[str]
) -> None:
    watch = PageWatch(Mock(spec=BrowserContext), "https://example.com")
    request = Mock(
        spec=Request,
        url="https://example.com/api/example?example=value",
        method="GET",
        resource_type="xhr",
        failure=failure,
    )
    watch.mark()
    watch._request(request)
    if status is not None:
        watch._response(Mock(spec=Response, request=request, status=status))
    watch._failed(request)

    assert watch.failures == expected, f"failure: {failure}, status: {status}"
    assert watch.pending_paths() == []


def test_external_requests_do_not_hold_the_page_busy_or_report_server_errors() -> None:
    watch = PageWatch(Mock(spec=BrowserContext), "https://example.com")
    request = Mock(
        spec=Request,
        url="https://external.example.com/example",
        method="GET",
        resource_type="fetch",
        failure="net::ERR_FAILED",
    )
    watch._request(request)
    watch._response(Mock(spec=Response, request=request, status=500))
    watch._failed(request)

    assert watch.pending_paths() == []
    assert watch.failures == []


def test_sentry_events_are_attributed_by_request_start_even_on_an_external_origin() -> None:
    watch = PageWatch(Mock(spec=BrowserContext), "https://example.com")
    event = b'{}\n{"type":"event"}\n{}'
    previous = Mock(
        spec=Request,
        url="https://external.example.com/api/1/envelope/",
        method="POST",
        resource_type="fetch",
        post_data_buffer=event,
    )
    watch._request(previous)
    watch.mark()
    watch._envelope(Mock(spec=Route, request=previous))
    assert watch.failures == []

    current = Mock(
        spec=Request,
        url="https://external.example.com/api/1/envelope/",
        method="POST",
        resource_type="fetch",
        post_data_buffer=event,
    )
    watch._request(current)
    watch._envelope(Mock(spec=Route, request=current))
    assert watch.failures == ["sentry_error: POST /api/1/envelope/"]
