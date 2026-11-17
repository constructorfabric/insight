"""admin.analytics.getFile over HTTP.

Slack answers a served day with a gzip file of JSON Lines and a refused day with
HTTP 200 and a JSON body `{"ok": false, "error": "<code>"}`. The two share a
status code, so the body decides: a refusal is raised as `SlackApiError` and
never reaches the stream as a record.

The file is streamed, never held whole: wire bytes are read in chunks, gunzipped
incrementally and parsed line by line, under hard limits on the compressed size,
the decompressed size and the line length. A large org's file costs one line of
memory; a decompression bomb stops at the limit.
"""

from __future__ import annotations

import itertools
import json
import logging
import math
import time
import zlib
from collections.abc import Iterable, Iterator, Mapping
from datetime import UTC, datetime
from email.utils import parsedate_to_datetime
from typing import Any

import requests

logger = logging.getLogger("airbyte")

API_URL = "https://slack.com/api/admin.analytics.getFile"
TIMEOUT = 60
MAX_RETRIES = 5
BACKOFF_BASE_SECS = 2.0
# A sync waits at most this long per retry, whatever Retry-After asks for.
MAX_WAIT_SECS = 300.0
_GZIP_MAGIC = b"\x1f\x8b"
_RETRY_STATUSES = {429, 500, 502, 503, 504}
# Refusals Slack itself calls temporary; retried like the HTTP statuses above.
RETRY_ERRORS = frozenset({"fatal_error", "internal_error", "ratelimited", "request_timeout", "service_unavailable"})

# Limits per day file. A member row is ~600 bytes, so 2 GiB is ~3.5M members: far
# above any Enterprise Grid, far below what a bomb would expand to.
MAX_COMPRESSED_BYTES = 512 * 1024**2
MAX_DECOMPRESSED_BYTES = 2 * 1024**3
MAX_LINE_BYTES = 1024**2
_READ_CHUNK = 64 * 1024
_INFLATE_STEP = 1024**2
# Transport Content-Encoding plus the file itself; anything deeper is not Slack.
_MAX_GZIP_LAYERS = 2


class SlackApiError(Exception):
    """Slack refused a request with `ok: false`."""

    def __init__(self, code: str, day: str) -> None:
        super().__init__(f"Slack refused admin.analytics.getFile for {day}: {code}")
        self.code = code
        self.day = day


class SlackResponseError(Exception):
    """Slack answered with something that is neither a member file nor a refusal.

    The message never quotes the body: a member file carries email addresses.
    """


def _retry_after(header: str | None) -> float | None:
    """Seconds from a Retry-After header: delay-seconds or an HTTP-date (RFC 9110 §10.2.3)."""
    if not header:
        return None
    try:
        seconds = float(header)
    except ValueError:
        try:
            when = parsedate_to_datetime(header)
        except (TypeError, ValueError):
            return None
        if when.tzinfo is None:
            return None
        seconds = (when - datetime.now(UTC)).total_seconds()
    if not math.isfinite(seconds):
        return None
    return max(seconds, 0.0)


def _wait(response: requests.Response | None, attempt: int) -> float:
    asked = _retry_after(response.headers.get("Retry-After")) if response is not None else None
    wait = asked if asked is not None else BACKOFF_BASE_SECS * 2**attempt
    return min(wait, MAX_WAIT_SECS)


def _wire(response: requests.Response, day: str) -> Iterator[bytes]:
    """Body bytes as sent, transport encoding included, under the compressed cap."""
    declared = response.headers.get("Content-Length")
    try:
        oversized = declared is not None and int(declared) > MAX_COMPRESSED_BYTES
    except ValueError:
        oversized = False  # a malformed header is left to the cap enforced while reading
    if oversized:
        raise SlackResponseError(f"{day}: body of {declared} bytes exceeds {MAX_COMPRESSED_BYTES}")
    total = 0
    while chunk := response.raw.read(_READ_CHUNK, decode_content=False):
        total += len(chunk)
        if total > MAX_COMPRESSED_BYTES:
            raise SlackResponseError(f"{day}: body exceeds {MAX_COMPRESSED_BYTES} bytes")
        yield chunk


def _gunzip(chunks: Iterable[bytes], day: str) -> Iterator[bytes]:
    """Inflate one gzip stream (members concatenated) in bounded steps."""
    inflater = zlib.decompressobj(wbits=31)
    started = False
    total = 0
    try:
        for chunk in chunks:
            data = chunk
            started = started or bool(data)
            while True:
                out = inflater.decompress(data, _INFLATE_STEP)
                total += len(out)
                if total > MAX_DECOMPRESSED_BYTES:
                    raise SlackResponseError(f"{day}: member file expands past {MAX_DECOMPRESSED_BYTES} bytes")
                if out:
                    yield out
                if inflater.eof:
                    data = inflater.unused_data
                    inflater = zlib.decompressobj(wbits=31)
                    started = bool(data)
                    if not data:
                        break
                    continue
                data = inflater.unconsumed_tail
                if not data and len(out) < _INFLATE_STEP:
                    break
    except zlib.error as error:
        raise SlackResponseError(f"{day}: corrupt gzip body ({error})") from error
    if started and not inflater.eof:
        raise SlackResponseError(f"{day}: gzip body ends mid-stream")


def _peel(chunks: Iterable[bytes], day: str) -> Iterator[bytes]:
    """Inflate as many gzip layers as the body carries, up to the limit."""
    for _ in range(_MAX_GZIP_LAYERS + 1):
        source = iter(chunks)
        head = b""
        for chunk in source:
            head += chunk
            if len(head) >= len(_GZIP_MAGIC):
                break
        chunks = itertools.chain([head], source)
        if not head.startswith(_GZIP_MAGIC):
            return (c for c in chunks if c)
        chunks = _gunzip(chunks, day)
    raise SlackResponseError(f"{day}: more than {_MAX_GZIP_LAYERS} gzip layers")


def _lines(chunks: Iterable[bytes], day: str) -> Iterator[bytes]:
    """Non-empty lines. Splitting and dropping empties run in C, so a body of bare
    newlines costs the decompressed cap in bytes, not a Python step per line."""
    tail = b""
    for chunk in chunks:
        *complete, tail = (tail + chunk).split(b"\n")
        for line in filter(None, complete):
            if len(line) > MAX_LINE_BYTES:
                raise SlackResponseError(f"{day}: a line exceeds {MAX_LINE_BYTES} bytes")
            yield line
        if len(tail) > MAX_LINE_BYTES:
            raise SlackResponseError(f"{day}: a line exceeds {MAX_LINE_BYTES} bytes")
    if tail:
        yield tail


def _parse(line: bytes, response: requests.Response, day: str) -> Any:
    try:
        return json.loads(line)
    except ValueError as error:
        raise SlackResponseError(
            f"{day}: body is not JSON Lines (Content-Type {response.headers.get('Content-Type')!r})"
        ) from error


def _rows(response: requests.Response, day: str) -> Iterator[dict[str, Any]]:
    """Rows of the member file, or the refusal it carries.

    A refusal is a single JSON object with `ok`; no member row has that key, so
    the first line decides.
    """
    first = True
    for line in _lines(_peel(_wire(response, day), day), day):
        if not line.strip():
            continue
        row = _parse(line, response, day)
        if not isinstance(row, Mapping):
            raise SlackResponseError(f"{day}: a member file line is not a JSON object")
        if first and "ok" in row:
            if row["ok"] is False:
                raise SlackApiError(str(row.get("error") or "unknown_error"), day)
            raise SlackResponseError(f"{day}: JSON answer without a member file")
        first = False
        yield dict(row)


class SlackAnalyticsClient:
    def __init__(self, token: str, session: requests.Session | None = None) -> None:
        self._session = session or requests.Session()
        self._session.headers["Authorization"] = f"Bearer {token}"

    def _get(self, day: str) -> requests.Response:
        """The day's response, headers read and body unread, after retrying what Slack calls temporary."""
        params = {"type": "member", "date": day}
        for attempt in itertools.count():
            try:
                response = self._session.get(API_URL, params=params, timeout=TIMEOUT, stream=True)
            except requests.RequestException as error:
                if attempt >= MAX_RETRIES:
                    raise
                logger.warning("getFile %s failed (%s), retrying", day, type(error).__name__)
                time.sleep(_wait(None, attempt))
                continue
            if response.status_code in _RETRY_STATUSES and attempt < MAX_RETRIES:
                wait = _wait(response, attempt)
                response.close()
                logger.warning("getFile %s answered HTTP %s, retrying in %.0fs", day, response.status_code, wait)
                time.sleep(wait)
                continue
            if response.status_code != 200:
                response.close()
                raise SlackResponseError(f"{day}: HTTP {response.status_code}")
            return response
        raise AssertionError("unreachable")

    def member_day(self, day: str) -> Iterator[dict[str, Any]]:
        """Every member row Slack reports for one UTC day, streamed."""
        for attempt in itertools.count():
            response = self._get(day)
            with response:
                rows = _rows(response, day)
                try:
                    first = next(rows, None)
                except SlackApiError as error:
                    if error.code not in RETRY_ERRORS or attempt >= MAX_RETRIES:
                        raise
                    wait = _wait(response, attempt)
                    logger.warning("%s, retrying in %.0fs", error, wait)
                    time.sleep(wait)
                    continue
                if first is not None:
                    yield first
                    yield from rows
                return
