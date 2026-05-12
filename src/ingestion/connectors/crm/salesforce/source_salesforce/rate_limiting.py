"""HTTP error classification and retry policy for Salesforce.

The CDK retry machinery hands every non-2xx response (and certain exceptions)
to ``SalesforceErrorHandler.interpret_response``, which decides retry vs fail
and — on ``INVALID_SESSION_ID`` — forces an OAuth token refresh before the
CDK retries.
"""

from __future__ import annotations

import logging
import re
import sys
from typing import TYPE_CHECKING, Any, Mapping, Optional, Union

import backoff
import requests
from requests import codes, exceptions  # type: ignore[import]

from airbyte_cdk.models import FailureType
from airbyte_cdk.sources.streams.http.error_handlers import (
    ErrorHandler,
    ErrorResolution,
    ResponseAction,
)
from airbyte_cdk.sources.streams.http.exceptions import DefaultBackoffException

from source_salesforce.exceptions import BulkNotSupportedException

if TYPE_CHECKING:
    from source_salesforce.api import SalesforceTokenProvider


# Urllib3/requests exceptions that occur mid-body-consumption. We've seen:
# - ProtocolError ("Connection broken: IncompleteRead") on self-hosted SF
# - ProtocolError ("InvalidChunkLength") on long cloud syncs
# - JSONDecodeError on malformed responses
# All are retried — replaying the request usually succeeds.
RESPONSE_CONSUMPTION_EXCEPTIONS = (
    exceptions.ChunkedEncodingError,
    exceptions.JSONDecodeError,
)

TRANSIENT_EXCEPTIONS = (
    DefaultBackoffException,
    exceptions.ConnectTimeout,
    exceptions.ReadTimeout,
    exceptions.ConnectionError,
    exceptions.HTTPError,
) + RESPONSE_CONSUMPTION_EXCEPTIONS

# 4xx status codes that are genuinely transient:
# - 406: empty-body responses occasionally observed in production; retry works.
# - 420: Salesforce Edge "setting things up" bootstrap page.
# 429 is handled explicitly below as a rate-limit failure.
_RETRYABLE_400_STATUS_CODES = {
    406,
    420,
}

_AUTHENTICATION_ERROR_MESSAGE_MAPPING = {
    "expired access/refresh token": (
        "The authentication to Salesforce has expired. "
        "Re-authenticate to restore access to Salesforce."
    )
}

logger = logging.getLogger("airbyte")


class SalesforceErrorHandler(ErrorHandler):
    """CDK error handler implementing Salesforce-specific retry and failure rules.

    Behavior:
    - 401 + INVALID_SESSION_ID  -> force token refresh + retry
    - 406/420/429 + 5xx         -> retry
    - 429/REQUEST_LIMIT_EXCEEDED on data endpoints -> FAIL (rolling 24h quota)
    - Bulk job creation errors classified into:
      * BulkNotSupportedException (caller falls back to REST)
      * Config error (permission / policy)
    - Login endpoint errors -> config error with remediation hint
    - TXN_SECURITY_METERING_ERROR -> config error with remediation hint
    """

    def __init__(
        self,
        stream_name: str = "<unknown stream>",
        sobject_options: Optional[Mapping[str, Any]] = None,
        token_provider: Optional["SalesforceTokenProvider"] = None,
    ) -> None:
        self._stream_name = stream_name
        self._sobject_options: Mapping[str, Any] = sobject_options or {}
        self._token_provider = token_provider

    @property
    def max_retries(self) -> Optional[int]:
        return 5

    @property
    def max_time(self) -> Optional[int]:
        return 120

    def interpret_response(
        self, response: Optional[Union[requests.Response, Exception]]
    ) -> ErrorResolution:
        if isinstance(response, TRANSIENT_EXCEPTIONS):
            return ErrorResolution(
                ResponseAction.RETRY,
                FailureType.transient_error,
                f"Error of type {type(response)} is transient. Retrying. ({response})",
            )

        if isinstance(response, requests.Response):
            if response.ok:
                if self._is_bulk_job_status_check(response):
                    try:
                        body = response.json()
                    except (ValueError, exceptions.JSONDecodeError):
                        body = {}
                    if body.get("state") == "Failed":
                        # A failed Bulk job is terminal; no retry. Surface as
                        # BulkNotSupported so the caller can try REST.
                        raise BulkNotSupportedException(
                            f"Query job with id: `{body.get('id')}` failed"
                        )
                return ErrorResolution(ResponseAction.IGNORE, None, None)

            if response.status_code == 401:
                error_code, _ = self._extract_error_code_and_message(response)
                if error_code == "INVALID_SESSION_ID":
                    if self._token_provider is None:
                        # Without a refresh provider, retrying replays the
                        # stale token forever. Fail so the caller surfaces a
                        # clear auth error instead of hiding it behind retries.
                        return ErrorResolution(
                            ResponseAction.FAIL,
                            FailureType.config_error,
                            "Salesforce session expired and no token refresh provider is wired for this handler.",
                        )
                    self._token_provider.force_refresh()
                    return ErrorResolution(
                        ResponseAction.RETRY,
                        FailureType.transient_error,
                        "Salesforce session expired; token refreshed, retrying.",
                    )

            if (
                not (400 <= response.status_code < 500)
                or response.status_code in _RETRYABLE_400_STATUS_CODES
            ):
                return ErrorResolution(
                    ResponseAction.RETRY,
                    FailureType.transient_error,
                    (
                        f"Response with status code {response.status_code} is transient. "
                        f"Retrying. ({response.content})"
                    ),
                )

            error_code, error_message = self._extract_error_code_and_message(response)

            if self._is_login_request(response):
                return ErrorResolution(
                    ResponseAction.FAIL,
                    FailureType.config_error,
                    (
                        _AUTHENTICATION_ERROR_MESSAGE_MAPPING.get(error_message)
                        if error_message in _AUTHENTICATION_ERROR_MESSAGE_MAPPING
                        else f"Salesforce login error: {response.content.decode()}"
                    ),
                )

            if response.status_code == codes.too_many_requests or (
                response.status_code == codes.forbidden
                and error_code == "REQUEST_LIMIT_EXCEEDED"
            ):
                # SF rate limit is rolling-24h; retrying immediately rarely helps
                # and we want clean sync-failed signals for ops. Check this before
                # the bulk-creation branch so a 403 REQUEST_LIMIT_EXCEEDED on the
                # Bulk create endpoint isn't reinterpreted as BulkNotSupported.
                return ErrorResolution(
                    ResponseAction.FAIL,
                    FailureType.transient_error,
                    f"Salesforce request limit reached (HTTP {response.status_code}): {response.text}",
                )

            if self._is_bulk_job_creation(response) and response.status_code in (
                codes.FORBIDDEN,
                codes.BAD_REQUEST,
            ):
                return self._handle_bulk_job_creation_endpoint_specific_errors(
                    response, error_code, error_message
                )

            if (
                "We can't complete the action because enabled transaction security"
                " policies took too long to complete." in error_message
                and error_code == "TXN_SECURITY_METERING_ERROR"
            ):
                return ErrorResolution(
                    ResponseAction.FAIL,
                    FailureType.config_error,
                    (
                        'Transaction security policy timeout. Assign the "Exempt '
                        'from Transaction Security" user permission to the '
                        "authenticated user to prevent future failures."
                    ),
                )

        return ErrorResolution(
            ResponseAction.FAIL,
            FailureType.system_error,
            f"Unhandled Salesforce error: {response.content.decode() if isinstance(response, requests.Response) else response}",
        )

    # ------- Response classification helpers ---------------------------------

    @staticmethod
    def _is_bulk_job_status_check(response: requests.Response) -> bool:
        """True iff the response is a Bulk 2.0 GET /jobs/query/{id} status call."""
        return (
            response.request.method == "GET"
            and bool(
                re.compile(r"/services/data/v\d{2}\.\d/jobs/query/[^/]+$").search(
                    response.url
                )
            )
        )

    @staticmethod
    def _is_bulk_job_creation(response: requests.Response) -> bool:
        return (
            response.request.method == "POST"
            and bool(
                re.compile(r"services/data/v\d{2}\.\d/jobs/query/?$").search(
                    response.url
                )
            )
        )

    def _handle_bulk_job_creation_endpoint_specific_errors(
        self,
        response: requests.Response,
        error_code: Optional[str],
        error_message: str,
    ) -> ErrorResolution:
        """Classify Bulk create failures into: REST-fallback vs. config-error.

        Three common failure modes (SF version-dependent):
        1) sobject genuinely not supported by Bulk API
        2) sobject not accessible to the Run-As user
        3) sobject not queryable directly (only as subquery)
        All three -> BulkNotSupportedException; caller switches to REST.

        Plus:
        - "Implementation restriction" -> config error (add View All Data)
        - LIMIT_EXCEEDED -> treat as Bulk-not-available for now
        - REQUEST_LIMIT_EXCEEDED -> same
        """
        if error_message == "Selecting compound data not supported in Bulk Query" or (
            error_code == "INVALIDENTITY"
            and "is not supported by the Bulk API" in error_message
        ):
            logger.error(
                f"Bulk API rejected '{self._stream_name}' "
                f"(options={self._sobject_options}): {error_message}"
            )
            raise BulkNotSupportedException()

        if response.status_code == codes.BAD_REQUEST:
            if error_message.endswith("does not support query"):
                logger.error(
                    f"'{self._stream_name}' is not queryable via Bulk "
                    f"(options={self._sobject_options}): {error_message}"
                )
                raise BulkNotSupportedException()
            if error_code == "API_ERROR" and error_message.startswith(
                "Implementation restriction"
            ):
                return ErrorResolution(
                    ResponseAction.FAIL,
                    FailureType.config_error,
                    (
                        f"Unable to sync '{self._stream_name}'. Grant the "
                        'authenticated user the "View All Data" permission.'
                    ),
                )
            if error_code == "LIMIT_EXCEEDED":
                logger.error(
                    "Salesforce API key has reached its 24h limit. "
                    "Replication will resume after the limit window elapses."
                )
                raise BulkNotSupportedException()

        if response.status_code == codes.FORBIDDEN:
            logger.error(
                f"Bulk API forbidden for '{self._stream_name}' "
                f"(options={self._sobject_options}, code={error_code}): {error_message}"
            )
            raise BulkNotSupportedException()

        return ErrorResolution(
            ResponseAction.FAIL, FailureType.system_error, error_message
        )

    @staticmethod
    def _extract_error_code_and_message(
        response: requests.Response,
    ) -> tuple[Optional[str], str]:
        try:
            error_data = response.json()[0]
            return error_data.get("errorCode"), error_data.get("message", "")
        except exceptions.JSONDecodeError:
            logger.warning(
                f"Response for `{response.request.url}` is not JSON: `{response.content}`"
            )
        except (IndexError, KeyError):
            logger.warning(
                f"Response for `{response.request.url}` was expected to be a "
                f"non-empty list but was `{response.content}`"
            )
            try:
                body = response.json()
                if "error" in body and "error_description" in body:
                    return body["error"], body["error_description"]
            except exceptions.JSONDecodeError:
                pass

        return None, f"Unknown error on response `{response.content}`"

    def _is_login_request(self, response: requests.Response) -> bool:
        return "/services/oauth2/token" in response.request.url


def default_backoff_handler(max_tries: int, retry_on=None):
    """Decorator factory for standalone request paths that aren't driven by CDK.

    Used for the OAuth login request and for direct describe() calls during
    stream construction — both happen before the CDK retry mechanism attaches.
    """
    if not retry_on:
        retry_on = TRANSIENT_EXCEPTIONS
    backoff_method = backoff.constant
    backoff_params = {"interval": 5}

    def log_retry_attempt(details):
        _, exc, _ = sys.exc_info()
        logger.info(str(exc))
        logger.info(
            f"Caught retryable error after {details['tries']} tries. "
            f"Waiting {details['wait']} seconds then retrying..."
        )

    def should_give_up(exc):
        response = getattr(exc, "response", None)
        give_up = (
            SalesforceErrorHandler()
            .interpret_response(response if response is not None else exc)
            .response_action
            != ResponseAction.RETRY
        )
        if give_up:
            if response is not None:
                logger.info(
                    "Giving up for returned HTTP status: %s, body: %s",
                    getattr(response, "status_code", "?"),
                    getattr(response, "text", "")[:500],
                )
            else:
                logger.info("Giving up for exception %s without response: %s", type(exc).__name__, exc)
        return give_up

    return backoff.on_exception(
        backoff_method,
        retry_on,
        jitter=None,
        on_backoff=log_retry_attempt,
        giveup=should_give_up,
        max_tries=max_tries,
        **backoff_params,
    )
