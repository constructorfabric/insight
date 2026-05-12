"""Availability strategy for Salesforce streams.

Some Salesforce sobjects become unavailable only at read time:
1. Access was restricted on a previously-synced object, so we now get 403.
2. The object doesn't support ``query``/``queryAll`` (only reachable as a
   subquery from a parent). These only surface at read.

Both are translated into a clean "skip this stream" result so one bad object
doesn't fail the whole sync.
"""

import logging
import typing
from typing import Optional, Tuple

from requests import HTTPError, JSONDecodeError, codes

from airbyte_cdk.sources.streams import Stream
from airbyte_cdk.sources.streams.http.availability_strategy import HttpAvailabilityStrategy


if typing.TYPE_CHECKING:
    from airbyte_cdk.sources import Source


class SalesforceAvailabilityStrategy(HttpAvailabilityStrategy):
    def handle_http_error(
        self,
        stream: Stream,
        logger: logging.Logger,
        source: Optional["Source"],
        error: HTTPError,
    ) -> Tuple[bool, Optional[str]]:
        if error.response.status_code not in (codes.FORBIDDEN, codes.BAD_REQUEST):
            raise error

        # Body may be non-JSON, empty, a dict, or a list.
        try:
            payload = error.response.json()
        except JSONDecodeError as json_error:
            raise error from json_error

        error_data: dict = {}
        if isinstance(payload, list) and payload and isinstance(payload[0], dict):
            error_data = payload[0]
        elif isinstance(payload, dict):
            error_data = payload

        error_code = error_data.get("errorCode") or ""
        message = error_data.get("message") or ""

        if error_code == "REQUEST_LIMIT_EXCEEDED":
            return False, (
                f"REQUEST_LIMIT_EXCEEDED: Salesforce org quota exceeded — skipping stream '{stream.name}': {message!r}"
            )
        return False, (
            f"Cannot receive data for stream '{stream.name}' (code={error_code!r}): {message!r}"
        )
