"""Compass connector test config builder and GraphQL request helpers.

Every stream POSTs to the same URL, so a matcher can only tell requests apart by
their body — and `HttpRequest` compares bodies exactly. That is a feature here:
the queries are read back out of `connector.yaml`, so a matcher asserts the
manifest's real query text, and passing `cursor=` asserts that the paginator
injected the cursor into `variables` rather than somewhere else.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

import yaml
from connector_tests import ConfigBuilder, HttpRequest

GRAPHQL_URL = "https://api.atlassian.com/graphql"
CLOUD_ID = "11111111-2222-3333-4444-555555555555"
SITE_SCOPE = f"ari:cloud:platform::site/{CLOUD_ID}"

_MANIFEST = yaml.safe_load((Path(__file__).resolve().parents[1] / "connector.yaml").read_text())


def component_ari(suffix: str) -> str:
    return f"ari:cloud:compass:{CLOUD_ID}:component/aaaaaaaa-0000-0000-0000-000000000000/{suffix}"


def scorecard_ari(suffix: str) -> str:
    return f"ari:cloud:compass:{CLOUD_ID}:scorecard/aaaaaaaa-0000-0000-0000-000000000000/{suffix}"


def team_ari(suffix: str) -> str:
    return f"ari:cloud:identity::team/{suffix}"


def user_ari(suffix: str) -> str:
    return f"ari:cloud:identity::user/{suffix}"


def _stream(name: str) -> dict:
    for stream in _MANIFEST["streams"]:
        if stream["name"] == name:
            return stream
    raise AssertionError(f"stream {name!r} is not in the manifest")


def _as_sent(query: str) -> str:
    """The manifest's query text as the CDK actually transmits it.

    A YAML block scalar keeps its trailing newline; interpolation drops it. The
    matcher compares bodies byte-exactly, so the newline has to go here too.
    """
    return query.rstrip("\n")


def child_query(stream: str) -> str:
    """The GraphQL document the stream itself sends."""
    return _as_sent(_stream(stream)["retriever"]["requester"]["request_body_json"]["query"])


def parent_query(stream: str) -> str:
    """The GraphQL document the stream's inline substream parent sends."""
    router = _stream(stream)["retriever"]["partition_router"]
    return _as_sent(
        router["parent_stream_configs"][0]["stream"]["retriever"]["requester"]["request_body_json"]["query"]
    )


def request(query: str, **variables: Any) -> HttpRequest:
    """A matcher for one GraphQL POST.

    Variables are compared exactly, so omitting `cursor` matches only the first
    page and passing it matches only the follow-up page.
    """
    return HttpRequest(GRAPHQL_URL, body={"query": query, "variables": variables})


class CompassConfigBuilder(ConfigBuilder):
    def __init__(self) -> None:
        super().__init__()
        self._config.update(
            {"atlassian_email": "bot@example.com", "atlassian_api_token": "test-token", "atlassian_cloud_id": CLOUD_ID}
        )
