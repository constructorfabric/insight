"""HTTP error classification and retry policy for HubSpot.

The CDK hands every non-2xx response (and certain exceptions) to
:class:`HubspotErrorHandler.interpret_response`, which decides retry vs fail.

HubSpot quirks covered here:
- 401 is terminal for Private App tokens (no session refresh to try).
- 403 MISSING_SCOPES is a permanent config error — fail fast with the missing
  scope list echoed to the operator.
- 429 responses sometimes lack a ``Retry-After`` header (especially on the
  search endpoint); fall back to a fixed delay and surface the fact.
- Cloudflare 530 indicates a malformed token — map to config error, not a
  transient retry.
"""

from __future__ import annotations

import logging
import time
from typing import Optional, Union

import requests
from requests import codes, exceptions  # type: ignore[import]

from airbyte_cdk.models import FailureType
from airbyte_cdk.sources.streams.http.error_handlers import (
    ErrorHandler,
    ErrorResolution,
    ResponseAction,
)
from airbyte_cdk.sources.streams.http.exceptions import DefaultBackoffException

from source_hubspot.constants import CLOUDFLARE_ORIGIN_DNS_ERROR


# Urllib3/requests exceptions that occur mid-body-consumption or at connect
# time. Replaying the request usually succeeds.
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
    *RESPONSE_CONSUMPTION_EXCEPTIONS,
)

# Fallback 429 sleep when HubSpot omits Retry-After. Search endpoint does this
# frequently. Slightly above the 4 rps search cap (1/4 = 0.25s) to avoid
# immediate re-trip.
_DEFAULT_RETRY_AFTER_SECONDS = 3
_SEARCH_RETRY_AFTER_SECONDS = 1.2

logger = logging.getLogger("airbyte")


def _is_search_request(response: requests.Response) -> bool:
    """True iff the response is a CRM Search POST."""
    try:
        return response.request.method == "POST" and "/crm/v3/objects/" in response.url and response.url.endswith("/search")
    except Exception:
        return False


class HubspotErrorHandler(ErrorHandler):
    """CDK error handler implementing HubSpot-specific retry and failure rules.

    Private App tokens are static — there is no INVALID_SESSION_ID equivalent
    worth retrying. A 401 means the operator revoked or mistyped the token,
    and the only useful action is to surface a config error.
    """

    max_retries: Optional[int] = 5
    max_time: Optional[int] = 600

    def __init__(self, stream_name: str = "<unknown stream>") -> None:
        self._stream_name = stream_name

    def interpret_response(
        self, response: Optional[Union[requests.Response, Exception]]
    ) -> ErrorResolution:
        if isinstance(response, TRANSIENT_EXCEPTIONS):
            return ErrorResolution(
                ResponseAction.RETRY,
                FailureType.transient_error,
                f"Error of type {type(response).__name__} is transient. Retrying. ({response})",
            )

        if isinstance(response, requests.Response):
            if response.ok:
                # SUCCESS, not IGNORE: IGNORE makes the CDK log
                # "Ignoring response for ..." at INFO on every 2xx, which
                # spams the log on busy syncs. SUCCESS is the happy-path
                # value; both let the body through to the parser unchanged.
                return ErrorResolution(ResponseAction.SUCCESS, None, None)

            status = response.status_code

            if status == codes.unauthorized:
                return ErrorResolution(
                    ResponseAction.FAIL,
                    FailureType.config_error,
                    (
                        f"HubSpot authentication failed (HTTP 401) on stream "
                        f"'{self._stream_name}'. Private App access token is "
                        "invalid or has been revoked — regenerate the token in "
                        "Settings → Integrations → Private Apps."
                    ),
                )

            if status == codes.forbidden:
                error_code, message = _extract_error(response)
                if error_code == "MISSING_SCOPES" or "MISSING_SCOPES" in message.upper():
                    missing = _extract_missing_scopes(response)
                    scope_hint = (
                        f" Missing scopes: {', '.join(missing)}." if missing else ""
                    )
                    return ErrorResolution(
                        ResponseAction.FAIL,
                        FailureType.config_error,
                        (
                            f"HubSpot Private App token is missing required "
                            f"scopes for stream '{self._stream_name}'.{scope_hint} "
                            "Grant the scopes in the Private App settings and retry."
                        ),
                    )
                return ErrorResolution(
                    ResponseAction.FAIL,
                    FailureType.config_error,
                    f"HubSpot access denied (HTTP 403) on '{self._stream_name}': {message or response.text[:300]}",
                )

            if status == codes.too_many_requests:
                delay = _parse_retry_after(response)
                logger.info(
                    "HubSpot 429 rate-limit on '%s'; sleeping %.1fs before retry (search=%s)",
                    self._stream_name,
                    delay,
                    _is_search_request(response),
                )
                # Sleep HERE so Retry-After is actually honoured.
                # ErrorResolution carries no delay slot — the CDK uses its own
                # backoff on top of ours, but that's acceptable.
                time.sleep(delay)
                return ErrorResolution(
                    ResponseAction.RATE_LIMITED,
                    FailureType.transient_error,
                    f"HubSpot rate limit reached (HTTP 429); slept {delay:.1f}s.",
                )

            if status == CLOUDFLARE_ORIGIN_DNS_ERROR:
                return ErrorResolution(
                    ResponseAction.FAIL,
                    FailureType.config_error,
                    (
                        "HubSpot returned Cloudflare 530 — the access token "
                        "format looks invalid. Private App tokens start with "
                        "'pat-' and are case-sensitive; regenerate if unsure."
                    ),
                )

            if status >= 500:
                return ErrorResolution(
                    ResponseAction.RETRY,
                    FailureType.transient_error,
                    f"HubSpot 5xx ({status}) on '{self._stream_name}'; retrying.",
                )

            # 4xx other than those handled above — surface with body for diagnosis.
            error_code, message = _extract_error(response)
            return ErrorResolution(
                ResponseAction.FAIL,
                FailureType.system_error,
                (
                    f"HubSpot error on '{self._stream_name}' (HTTP {status}, "
                    f"code={error_code}): {message or response.text[:500]}"
                ),
            )

        return ErrorResolution(
            ResponseAction.FAIL,
            FailureType.system_error,
            f"Unhandled HubSpot error on '{self._stream_name}': {response!r}",
        )


def _parse_retry_after(response: requests.Response) -> float:
    header = response.headers.get("Retry-After")
    if header:
        try:
            return float(header) + 1.0  # safety margin
        except (TypeError, ValueError):
            pass
    if _is_search_request(response):
        return _SEARCH_RETRY_AFTER_SECONDS
    return float(_DEFAULT_RETRY_AFTER_SECONDS)


def _extract_error(response: requests.Response) -> tuple[Optional[str], str]:
    try:
        body = response.json()
    except (exceptions.JSONDecodeError, ValueError):
        return None, response.text[:500] if response.text else ""
    if isinstance(body, dict):
        return body.get("category") or body.get("errorCode") or body.get("error"), (
            body.get("message") or body.get("error_description") or ""
        )
    return None, str(body)[:500]


def _extract_missing_scopes(response: requests.Response) -> list[str]:
    try:
        body = response.json()
    except (exceptions.JSONDecodeError, ValueError):
        return []
    if not isinstance(body, dict):
        return []
    errors = body.get("errors") or []
    if errors and isinstance(errors, list) and isinstance(errors[0], dict):
        ctx = errors[0].get("context") or {}
        scopes = ctx.get("requiredScopes")
        if isinstance(scopes, list):
            return [str(s) for s in scopes]
    context = body.get("context") or {}
    if isinstance(context, dict):
        scopes = context.get("requiredScopes")
        if isinstance(scopes, list):
            return [str(s) for s in scopes]
    return []


