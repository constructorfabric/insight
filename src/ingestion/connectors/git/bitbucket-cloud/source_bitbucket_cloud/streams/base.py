"""Base stream for Bitbucket Cloud REST API v2.0.

Stock Airbyte CDK primitives only. No custom read_records overrides, no
response-swallowing, no blocking sleeps. Errors raise by default and CDK
retries via should_retry/backoff_time.
"""

import logging
import random
from abc import ABC
from datetime import datetime, timezone
from typing import Any, Iterable, Mapping, MutableMapping, Optional

import requests
from airbyte_cdk.sources.streams.http import HttpStream
from airbyte_cdk.sources.streams.http.error_handlers import (
    ErrorHandler,
    HttpStatusErrorHandler,
)
from airbyte_cdk.sources.streams.http.error_handlers.default_error_mapping import (
    DEFAULT_ERROR_MAPPING,
)
from airbyte_cdk.sources.streams.http.error_handlers.response_models import (
    ErrorResolution,
    ResponseAction,
)
from airbyte_cdk.models import FailureType

from source_bitbucket_cloud.auth import auth_headers

_logger = logging.getLogger("airbyte")


def _now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _normalize_start_date(value: Optional[str]) -> Optional[str]:
    """Accept YYYY-MM-DD or full ISO 8601; return YYYY-MM-DD.

    start_date is compared against `record_timestamp[:10]` in streams, so the
    RHS must be exactly 10 chars. Caller may hand us a bare date or a full
    ISO timestamp — normalize once at startup so the comparison is sound.
    Raises ValueError on unparseable input: fail config rather than drift cursors.
    """
    if not value:
        return None
    if len(value) == 10 and value[4] == "-" and value[7] == "-":
        datetime.strptime(value, "%Y-%m-%d")
        return value
    dt = datetime.fromisoformat(value.replace("Z", "+00:00"))
    return dt.date().isoformat()


def _make_unique_key(tenant_id: str, source_id: str, *parts: str) -> str:
    return f"{tenant_id}:{source_id}:{':'.join(parts)}"


_TRUNCATE_SUFFIX = "…[truncated]"
_TRUNCATE_LIMIT = 1_024  # 1 KB UTF-8


def _truncate(text: Optional[str], limit: int = _TRUNCATE_LIMIT) -> Optional[str]:
    """Cap text at `limit` UTF-8 bytes, appending a suffix when cut.

    Bitbucket PR bodies, commit messages and review comments are unbounded;
    a pathological record would otherwise balloon destination aggregation
    buffers and OOM the ClickHouse pod (job 89 root cause).
    """
    if text is None:
        return None
    suffix = _TRUNCATE_SUFFIX
    suffix_bytes = suffix.encode("utf-8")
    budget = limit - len(suffix_bytes)
    if budget <= 0:
        # Limit smaller than suffix itself — byte-slice the suffix directly.
        return suffix_bytes[:limit].decode("utf-8", errors="ignore")
    encoded = text.encode("utf-8", errors="replace")
    if len(encoded) <= limit:
        return text
    # Trim dangling partial multi-byte char.
    return encoded[:budget].decode("utf-8", errors="ignore") + suffix


class BitbucketCloudStream(HttpStream, ABC):
    """Base for all Bitbucket Cloud streams.

    Keeps request/response handling as close to CDK defaults as possible:
    - raise_on_http_errors stays True (default) — exhausted retries fail the stream.
    - should_retry only distinguishes retryable vs terminal status codes.
    - backoff_time honours Retry-After on 429 and a fixed 30s on transient 5xx.
    """

    url_base = "https://api.bitbucket.org/2.0/"
    primary_key = "unique_key"
    page_size = 100

    # Sub-streams that iterate per-PR or per-commit slices set this True so
    # an orphaned 404 (PR references deleted branch, commit diffstat gone)
    # skips the one slice instead of failing the whole stream.
    ignore_404: bool = False

    def __init__(
        self,
        token: str,
        tenant_id: str,
        source_id: str,
        username: str = "",
        **kwargs: Any,
    ) -> None:
        super().__init__(**kwargs)
        self._token = token
        self._username = username
        self._tenant_id = tenant_id
        self._source_id = source_id

    # ------------------------------------------------------------------
    # Requests
    # ------------------------------------------------------------------

    def request_headers(self, **kwargs: Any) -> Mapping[str, Any]:
        return auth_headers(self._token, self._username)

    def request_params(
        self,
        stream_state: Optional[Mapping[str, Any]] = None,
        stream_slice: Optional[Mapping[str, Any]] = None,
        next_page_token: Optional[Mapping[str, Any]] = None,
    ) -> MutableMapping[str, Any]:
        # When following a next_url the URL already carries the params.
        if next_page_token:
            return {}
        return {"pagelen": str(self.page_size)}

    def next_page_token(self, response: requests.Response) -> Optional[Mapping[str, Any]]:
        try:
            data = response.json()
        except ValueError:
            return None
        nxt = data.get("next")
        return {"next_url": nxt} if nxt else None

    def path(
        self,
        stream_state: Optional[Mapping[str, Any]] = None,
        stream_slice: Optional[Mapping[str, Any]] = None,
        next_page_token: Optional[Mapping[str, Any]] = None,
    ) -> str:
        if next_page_token and "next_url" in next_page_token:
            full_url = next_page_token["next_url"]
            if full_url.startswith(self.url_base):
                return full_url[len(self.url_base):]
            return full_url.replace("https://api.bitbucket.org/2.0/", "")
        return self._path(stream_slice=stream_slice)

    def _path(self, stream_slice: Optional[Mapping[str, Any]] = None) -> str:
        raise NotImplementedError

    # ------------------------------------------------------------------
    # Retry
    # ------------------------------------------------------------------

    def should_retry(self, response: requests.Response) -> bool:
        # 401/403/404 are terminal — do not retry and do not silently swallow.
        if response.status_code in (401, 403, 404):
            _logger.warning(
                f"{self.name}: terminal {response.status_code} on {response.url} "
                f"body={response.text[:200]!r}"
            )
            return False
        retry = response.status_code in (429, 500, 502, 503, 504)
        if retry:
            _logger.warning(
                f"{self.name}: retryable {response.status_code} on {response.url}"
            )
        return retry

    def get_error_handler(self) -> Optional[ErrorHandler]:
        if not self.ignore_404:
            return super().get_error_handler()
        mapping = {
            **DEFAULT_ERROR_MAPPING,
            404: ErrorResolution(
                response_action=ResponseAction.IGNORE,
                failure_type=FailureType.transient_error,
                error_message="404: resource missing, skipping slice",
            ),
        }
        return HttpStatusErrorHandler(
            logger=_logger,
            error_mapping=mapping,
            max_retries=self.max_retries,
        )

    def backoff_time(self, response: requests.Response) -> Optional[float]:
        if response.status_code == 429:
            retry_after = response.headers.get("Retry-After")
            try:
                wait = max(float(retry_after), 1.0) if retry_after else 60.0
            except (TypeError, ValueError):
                wait = 60.0
            _logger.warning(
                f"{self.name}: 429 throttled, backing off {wait}s "
                f"(Retry-After={retry_after!r})"
            )
            return wait
        if response.status_code in (500, 502, 503, 504):
            # Honor Retry-After on 5xx if the origin/CDN supplies one; otherwise
            # 30s + small jitter to avoid thundering-herd on recovery.
            retry_after = response.headers.get("Retry-After")
            if retry_after:
                try:
                    wait = max(float(retry_after), 1.0)
                except (TypeError, ValueError):
                    wait = 30.0 + random.uniform(0, 10)
            else:
                wait = 30.0 + random.uniform(0, 10)
            _logger.warning(
                f"{self.name}: {response.status_code}, backing off {wait:.1f}s "
                f"(Retry-After={retry_after!r})"
            )
            return wait
        return None

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------

    def _envelope(self, record: Mapping[str, Any]) -> MutableMapping[str, Any]:
        out = dict(record)
        out["tenant_id"] = self._tenant_id
        out["source_id"] = self._source_id
        out["data_source"] = "insight_bitbucket_cloud"
        out["collected_at"] = _now_iso()
        return out

    def _iter_values(self, response: requests.Response) -> Iterable[Mapping[str, Any]]:
        try:
            data = response.json()
        except ValueError:
            _logger.warning(
                f"{self.name}: non-JSON response {response.status_code} on {response.url}"
            )
            return []
        values = data.get("values", []) or []
        _logger.debug(
            f"{self.name}: {len(values)} values on page (url={response.url})"
        )
        return values
