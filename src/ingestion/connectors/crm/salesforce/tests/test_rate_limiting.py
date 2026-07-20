"""Tests for source_salesforce.rate_limiting: error classification + backoff."""

from __future__ import annotations

import time
from unittest.mock import Mock

import pytest
from airbyte_cdk.models import FailureType
from airbyte_cdk.sources.streams.http.error_handlers import ResponseAction
from requests import exceptions
from source_salesforce.rate_limiting import SalesforceErrorHandler, default_backoff_handler
from tests.conftest import make_http_response

LOGIN_URL = "https://x.example/services/oauth2/token"


# ---------------------------------------------------------------------------
# SalesforceErrorHandler.interpret_response
# ---------------------------------------------------------------------------


class TestRetryBudget:
    def test_max_retries_and_max_time(self):
        handler = SalesforceErrorHandler()
        assert handler.max_retries == 5
        assert handler.max_time == 120


class TestTransientExceptions:
    @pytest.mark.parametrize(
        "exc",
        [
            exceptions.ConnectionError("refused"),
            exceptions.ConnectTimeout("slow"),
            exceptions.ReadTimeout("slow"),
            exceptions.ChunkedEncodingError("IncompleteRead"),
        ],
    )
    def test_retried(self, exc):
        resolution = SalesforceErrorHandler().interpret_response(exc)
        assert resolution.response_action == ResponseAction.RETRY
        assert resolution.failure_type == FailureType.transient_error


class TestResponses:
    def test_ok_response_ignored(self):
        resolution = SalesforceErrorHandler().interpret_response(make_http_response(200))
        assert resolution.response_action == ResponseAction.IGNORE

    def test_401_invalid_session_refreshes_and_retries(self):
        provider = Mock()
        handler = SalesforceErrorHandler(token_provider=provider)
        resp = make_http_response(401, [{"errorCode": "INVALID_SESSION_ID", "message": "Session expired"}])
        resolution = handler.interpret_response(resp)
        provider.force_refresh.assert_called_once()
        assert resolution.response_action == ResponseAction.RETRY

    def test_401_invalid_session_without_provider_fails(self):
        resp = make_http_response(401, [{"errorCode": "INVALID_SESSION_ID", "message": "Session expired"}])
        resolution = SalesforceErrorHandler().interpret_response(resp)
        assert resolution.response_action == ResponseAction.FAIL
        assert resolution.failure_type == FailureType.config_error
        assert "no token refresh provider" in resolution.error_message

    def test_401_other_code_fails_as_config_error(self):
        resp = make_http_response(401, [{"errorCode": "INVALID_AUTH_HEADER", "message": "nope"}])
        resolution = SalesforceErrorHandler(token_provider=Mock()).interpret_response(resp)
        assert resolution.response_action == ResponseAction.FAIL
        assert resolution.failure_type == FailureType.config_error
        assert "INVALID_AUTH_HEADER" in resolution.error_message

    @pytest.mark.parametrize("status", [406, 420, 500, 503])
    def test_transient_status_codes_retried(self, status):
        resolution = SalesforceErrorHandler().interpret_response(make_http_response(status))
        assert resolution.response_action == ResponseAction.RETRY

    def test_login_error_is_config_error(self):
        resp = make_http_response(400, {"error": "invalid_grant", "error_description": "bad creds"}, url=LOGIN_URL)
        resolution = SalesforceErrorHandler().interpret_response(resp)
        assert resolution.response_action == ResponseAction.FAIL
        assert resolution.failure_type == FailureType.config_error
        assert "Salesforce login error" in resolution.error_message

    def test_login_expired_token_message_mapped(self):
        resp = make_http_response(
            400, {"error": "invalid_grant", "error_description": "expired access/refresh token"}, url=LOGIN_URL
        )
        resolution = SalesforceErrorHandler().interpret_response(resp)
        assert "Re-authenticate to restore access" in resolution.error_message

    def test_429_fails_hard(self):
        resolution = SalesforceErrorHandler().interpret_response(
            make_http_response(429, [{"errorCode": "REQUEST_LIMIT_EXCEEDED", "message": "q"}])
        )
        assert resolution.response_action == ResponseAction.FAIL
        assert "request limit reached" in resolution.error_message

    def test_403_request_limit_exceeded_fails_hard(self):
        resolution = SalesforceErrorHandler().interpret_response(
            make_http_response(403, [{"errorCode": "REQUEST_LIMIT_EXCEEDED", "message": "q"}])
        )
        assert resolution.response_action == ResponseAction.FAIL

    def test_403_other_code_is_system_error(self):
        resolution = SalesforceErrorHandler().interpret_response(
            make_http_response(403, [{"errorCode": "API_DISABLED", "message": "no api"}])
        )
        assert resolution.response_action == ResponseAction.FAIL
        assert resolution.failure_type == FailureType.system_error

    def test_txn_security_metering_is_config_error(self):
        message = (
            "We can't complete the action because enabled transaction security policies took too long to complete."
        )
        resolution = SalesforceErrorHandler().interpret_response(
            make_http_response(400, [{"errorCode": "TXN_SECURITY_METERING_ERROR", "message": message}])
        )
        assert resolution.response_action == ResponseAction.FAIL
        assert resolution.failure_type == FailureType.config_error
        assert "Exempt " in resolution.error_message

    def test_unhandled_400_is_system_error(self):
        resolution = SalesforceErrorHandler().interpret_response(
            make_http_response(400, [{"errorCode": "MALFORMED_QUERY", "message": "bad"}])
        )
        assert resolution.response_action == ResponseAction.FAIL
        assert resolution.failure_type == FailureType.system_error

    def test_unknown_exception_is_system_error(self):
        resolution = SalesforceErrorHandler().interpret_response(ValueError("weird"))
        assert resolution.response_action == ResponseAction.FAIL
        assert resolution.failure_type == FailureType.system_error


class TestExtractErrorCodeAndMessage:
    def test_json_list_payload(self):
        resp = make_http_response(400, [{"errorCode": "X", "message": "boom"}])
        assert SalesforceErrorHandler._extract_error_code_and_message(resp) == ("X", "boom")

    def test_non_json_body(self):
        resp = make_http_response(400, content=b"<html>oops</html>")
        code, message = SalesforceErrorHandler._extract_error_code_and_message(resp)
        assert code is None
        assert "Unknown error" in message

    def test_oauth_dict_payload(self):
        resp = make_http_response(400, {"error": "invalid_grant", "error_description": "bad"})
        assert SalesforceErrorHandler._extract_error_code_and_message(resp) == ("invalid_grant", "bad")

    def test_dict_without_oauth_keys(self):
        resp = make_http_response(400, {"detail": "something"})
        code, message = SalesforceErrorHandler._extract_error_code_and_message(resp)
        assert code is None
        assert "Unknown error" in message

    def test_empty_list_payload(self):
        resp = make_http_response(400, [])
        code, message = SalesforceErrorHandler._extract_error_code_and_message(resp)
        assert code is None
        assert "Unknown error" in message


# ---------------------------------------------------------------------------
# default_backoff_handler
# ---------------------------------------------------------------------------


class TestDefaultBackoffHandler:
    @pytest.fixture(autouse=True)
    def _no_sleep(self, monkeypatch):
        monkeypatch.setattr(time, "sleep", lambda seconds: None)

    def test_retries_transient_then_succeeds(self):
        calls = []

        @default_backoff_handler(max_tries=4)
        def flaky():
            calls.append(1)
            if len(calls) < 3:
                raise exceptions.ConnectionError("blip")
            return "ok"

        assert flaky() == "ok"
        assert len(calls) == 3

    def test_gives_up_on_non_retryable_response(self):
        resp = make_http_response(401, [{"errorCode": "INVALID_AUTH_HEADER", "message": "no"}])
        calls = []

        @default_backoff_handler(max_tries=5)
        def failing():
            calls.append(1)
            raise exceptions.HTTPError("401", response=resp)

        with pytest.raises(exceptions.HTTPError):
            failing()
        assert len(calls) == 1  # no retries: handler said FAIL

    def test_gives_up_after_max_tries(self):
        calls = []

        @default_backoff_handler(max_tries=2)
        def always_down():
            calls.append(1)
            raise exceptions.ConnectionError("down")

        with pytest.raises(exceptions.ConnectionError):
            always_down()
        assert len(calls) == 2

    def test_gives_up_on_exception_without_response(self):
        """A non-transient exception (no .response) stops retries immediately."""
        calls = []

        @default_backoff_handler(max_tries=5, retry_on=(ValueError,))
        def broken():
            calls.append(1)
            raise ValueError("logic bug, not transient")

        with pytest.raises(ValueError):
            broken()
        assert len(calls) == 1

    def test_custom_retry_on(self):
        calls = []

        @default_backoff_handler(max_tries=3, retry_on=(exceptions.ConnectionError,))
        def down_then_up():
            calls.append(1)
            if len(calls) == 1:
                raise exceptions.ConnectionError("blip")
            return "ok"

        assert down_then_up() == "ok"
        assert len(calls) == 2
