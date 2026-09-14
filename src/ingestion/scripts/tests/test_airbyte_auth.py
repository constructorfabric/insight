"""Only an http(s) endpoint may receive the Airbyte credentials.

AIRBYTE_URL is a workflow parameter, so it is parsed at the boundary: any
other scheme (file://, ftp://, a bare path) is refused before a token or a
client secret goes anywhere.

Run: pytest src/ingestion/scripts/tests
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from airbyte_auth import FatalError, airbyte_api_url


@pytest.mark.parametrize("url", ["http://airbyte.example.internal:8001", "https://airbyte.example.internal"])
def test_http_s_urls_pass_with_trailing_slash_trimmed(monkeypatch, url) -> None:
    monkeypatch.setenv("AIRBYTE_URL", url + "/")

    assert airbyte_api_url() == url, f"should accept: {url!r}"


@pytest.mark.parametrize(
    "url", ["file:///etc/passwd", "ftp://airbyte.example.internal", "airbyte.example.internal:8001", ""]
)
def test_anything_else_is_refused_before_credentials_move(monkeypatch, url) -> None:
    monkeypatch.setenv("AIRBYTE_URL", url)

    with pytest.raises(FatalError):
        airbyte_api_url()
