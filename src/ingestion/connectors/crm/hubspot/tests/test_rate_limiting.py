"""HubspotErrorHandler: retry/fail classification, Retry-After parsing."""

from __future__ import annotations

import json

import pytest
import requests
from airbyte_cdk.models import FailureType
from airbyte_cdk.sources.streams.http.error_handlers import ResponseAction
from airbyte_cdk.sources.streams.http.exceptions import DefaultBackoffException
from source_hubspot import rate_limiting as rl
from source_hubspot.rate_limiting import (
    HubspotErrorHandler,
    _extract_error,
    _extract_missing_scopes,
    _is_search_request,
    _parse_retry_after,
)

SEARCH_URL = "https://api.hubapi.com/crm/v3/objects/deals/search"
LIST_URL = "https://api.hubapi.com/crm/v3/owners/"


def make_response(status=200, body=None, headers=None, url=LIST_URL, method="GET", text=None):
    """Real requests.Response so isinstance checks in the handler hold."""
    resp = requests.Response()
    resp.status_code = status
    if text is not None:
        resp._content = text.encode("utf-8")
    elif body is not None:
        resp._content = json.dumps(body).encode("utf-8")
    else:
        resp._content = b""
    resp.headers.update(headers or {})
    resp.url = url
    req = requests.PreparedRequest()
    req.method = method
    req.url = url
    resp.request = req
    return resp


@pytest.fixture
def handler() -> HubspotErrorHandler:
    return HubspotErrorHandler("deals")


@pytest.fixture(autouse=True)
def no_sleep(monkeypatch):
    """429 handling sleeps inline — record instead of blocking the suite."""
    slept: list[float] = []
    monkeypatch.setattr(rl.time, "sleep", slept.append)
    return slept


class TestTransientExceptions:
    @pytest.mark.parametrize(
        "exc",
        [
            requests.exceptions.ConnectionError("refused"),
            requests.exceptions.ReadTimeout("slow"),
            requests.exceptions.ChunkedEncodingError("cut"),
            DefaultBackoffException(
                request=requests.PreparedRequest(), response=requests.Response(), error_message="x"
            ),
        ],
    )
    def test_retry(self, handler, exc):
        resolution = handler.interpret_response(exc)
        assert resolution.response_action == ResponseAction.RETRY
        assert resolution.failure_type == FailureType.transient_error


class TestHttpStatuses:
    def test_2xx_is_success(self, handler):
        resolution = handler.interpret_response(make_response(200))
        assert resolution.response_action == ResponseAction.SUCCESS
        assert resolution.failure_type is None

    def test_401_is_config_error(self, handler):
        resolution = handler.interpret_response(make_response(401))
        assert resolution.response_action == ResponseAction.FAIL
        assert resolution.failure_type == FailureType.config_error
        assert "deals" in resolution.error_message
        assert "Private App" in resolution.error_message

    def test_403_missing_scopes_lists_scopes(self, handler):
        body = {
            "category": "MISSING_SCOPES",
            "message": "denied",
            "errors": [{"context": {"requiredScopes": ["crm.objects.deals.read"]}}],
        }
        resolution = handler.interpret_response(make_response(403, body=body))
        assert resolution.response_action == ResponseAction.FAIL
        assert resolution.failure_type == FailureType.config_error
        assert "crm.objects.deals.read" in resolution.error_message

    def test_403_missing_scopes_in_message_text(self, handler):
        body = {"category": "OTHER", "message": "missing_scopes detected"}
        resolution = handler.interpret_response(make_response(403, body=body))
        assert "missing required" in resolution.error_message
        # No scope list in the body → no scope hint.
        assert "Missing scopes:" not in resolution.error_message

    def test_403_generic_denied(self, handler):
        body = {"category": "BANNED", "message": "account suspended"}
        resolution = handler.interpret_response(make_response(403, body=body))
        assert resolution.failure_type == FailureType.config_error
        assert "account suspended" in resolution.error_message

    def test_429_honours_retry_after_header(self, handler, no_sleep):
        resolution = handler.interpret_response(make_response(429, headers={"Retry-After": "7"}))
        assert resolution.response_action == ResponseAction.RATE_LIMITED
        assert resolution.failure_type == FailureType.transient_error
        assert no_sleep == [8.0]  # header + 1s safety margin

    def test_429_search_fallback_delay(self, handler, no_sleep):
        handler.interpret_response(make_response(429, url=SEARCH_URL, method="POST"))
        assert no_sleep == [1.2]

    def test_429_default_fallback_delay(self, handler, no_sleep):
        handler.interpret_response(make_response(429))
        assert no_sleep == [3.0]

    def test_530_is_token_format_config_error(self, handler):
        resolution = handler.interpret_response(make_response(530))
        assert resolution.response_action == ResponseAction.FAIL
        assert resolution.failure_type == FailureType.config_error
        assert "pat-" in resolution.error_message

    @pytest.mark.parametrize("status", [500, 502, 503])
    def test_5xx_retries(self, handler, status):
        resolution = handler.interpret_response(make_response(status))
        assert resolution.response_action == ResponseAction.RETRY
        assert resolution.failure_type == FailureType.transient_error

    def test_other_4xx_fails_with_body(self, handler):
        body = {"category": "VALIDATION_ERROR", "message": "bad filter"}
        resolution = handler.interpret_response(make_response(400, body=body))
        assert resolution.response_action == ResponseAction.FAIL
        assert resolution.failure_type == FailureType.system_error
        assert "VALIDATION_ERROR" in resolution.error_message
        assert "bad filter" in resolution.error_message

    def test_unhandled_input_fails(self, handler):
        resolution = handler.interpret_response(ValueError("surprise"))
        assert resolution.response_action == ResponseAction.FAIL
        assert resolution.failure_type == FailureType.system_error
        assert "Unhandled" in resolution.error_message


class TestIsSearchRequest:
    def test_post_to_search_endpoint(self):
        assert _is_search_request(make_response(url=SEARCH_URL, method="POST")) is True

    def test_get_is_not_search(self):
        assert _is_search_request(make_response(url=SEARCH_URL, method="GET")) is False

    def test_non_search_url(self):
        assert _is_search_request(make_response(url=LIST_URL, method="POST")) is False

    def test_missing_request_swallowed(self):
        resp = requests.Response()  # .request is None → AttributeError inside
        resp.url = SEARCH_URL
        assert _is_search_request(resp) is False


class TestParseRetryAfter:
    def test_numeric_header_plus_margin(self):
        resp = make_response(429, headers={"Retry-After": "2.5"})
        assert _parse_retry_after(resp) == 3.5

    def test_invalid_header_falls_back(self):
        resp = make_response(429, headers={"Retry-After": "soon"})
        assert _parse_retry_after(resp) == 3.0

    def test_invalid_header_on_search_falls_back_to_search_delay(self):
        resp = make_response(429, headers={"Retry-After": "soon"}, url=SEARCH_URL, method="POST")
        assert _parse_retry_after(resp) == 1.2


class TestExtractError:
    def test_json_dict_fields(self):
        resp = make_response(400, body={"category": "X", "message": "m"})
        assert _extract_error(resp) == ("X", "m")

    def test_error_code_and_description_fallbacks(self):
        resp = make_response(400, body={"errorCode": "E1", "error_description": "d"})
        assert _extract_error(resp) == ("E1", "d")

    def test_non_json_body(self):
        resp = make_response(400, text="<html>oops</html>")
        assert _extract_error(resp) == (None, "<html>oops</html>")

    def test_empty_body(self):
        resp = make_response(400, text="")
        assert _extract_error(resp) == (None, "")

    def test_non_dict_json(self):
        resp = make_response(400, body=["a", "b"])
        assert _extract_error(resp) == (None, "['a', 'b']")


class TestExtractMissingScopes:
    def test_scopes_from_errors_context(self):
        resp = make_response(403, body={"errors": [{"context": {"requiredScopes": ["s1", "s2"]}}]})
        assert _extract_missing_scopes(resp) == ["s1", "s2"]

    def test_scopes_from_top_level_context(self):
        resp = make_response(403, body={"context": {"requiredScopes": ["s3"]}})
        assert _extract_missing_scopes(resp) == ["s3"]

    def test_non_list_scopes_ignored(self):
        resp = make_response(403, body={"context": {"requiredScopes": "s3"}})
        assert _extract_missing_scopes(resp) == []

    def test_non_dict_body(self):
        resp = make_response(403, body=["nope"])
        assert _extract_missing_scopes(resp) == []

    def test_non_json_body(self):
        resp = make_response(403, text="<html>")
        assert _extract_missing_scopes(resp) == []
