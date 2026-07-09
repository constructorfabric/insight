"""Shared fixtures/helpers for source_github_copilot unit tests.

All tests are offline: HTTP is stubbed via FakeResponse (a duck-typed
requests.Response). No network, no credentials.
"""

from __future__ import annotations

from typing import Any, Iterable

import requests

BASE_CONFIG: dict[str, Any] = {
    "insight_tenant_id": "T",
    "insight_source_id": "S",
    "github_token": "tok",
    "github_org": "acme",
}

# Kwargs shape expected by CopilotRestStream.__init__ / SourceGitHubCopilot.streams().
SHARED_STREAM_KWARGS: dict[str, Any] = {
    "token": "tok",
    "tenant_id": "T",
    "source_id": "S",
    "org": "acme",
}


class FakeResponse:
    """Duck-typed requests.Response for parse_response/next_page_token/_guard_response.

    `json_error`, when set, makes `.json()` raise it (simulates a non-JSON body).
    `lines` feeds `.iter_lines()` for the NDJSON signed-URL download path.
    """

    def __init__(
        self,
        payload: Any = None,
        status_code: int = 200,
        headers: dict | None = None,
        links: dict | None = None,
        text: str = "",
        lines: list[str] | None = None,
        url: str = "https://api.github.com/x",
        json_error: Exception | None = None,
    ):
        self._payload = payload
        self._json_error = json_error
        self.status_code = status_code
        self.headers = headers or {}
        self.links = links or {}
        self.text = text
        self._lines = lines if lines is not None else []
        self.url = url

    def json(self) -> Any:
        if self._json_error is not None:
            raise self._json_error
        return self._payload

    def iter_lines(self, decode_unicode: bool = True) -> Iterable[str]:
        yield from self._lines


def real_response(
    status_code: int = 200,
    headers: dict | None = None,
    content: bytes = b"",
    url: str = "https://api.github.com/x",
) -> requests.Response:
    """A genuine requests.Response — needed wherever connector code does
    `isinstance(response, requests.Response)` (should_retry / backoff_time /
    seats.next_page_token), which a duck-typed FakeResponse would fail."""
    resp = requests.Response()
    resp.status_code = status_code
    resp.headers.update(headers or {})
    resp._content = content
    resp.encoding = "utf-8"
    resp.url = url
    return resp
