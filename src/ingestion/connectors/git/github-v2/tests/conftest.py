"""Shared fixtures for source_github_v2 unit tests.

All tests are offline: HTTP responses are stubbed via FakeResponse and
stream methods are exercised directly (parse_response, _variables,
stream_slices, get_updated_state). Where a stream delegates to the CDK
(``super().read_records()``), ``HttpStream.read_records`` is monkeypatched.
No network, no credentials.
"""

from __future__ import annotations

import json
from collections.abc import Iterable, Mapping
from typing import Any

import pytest
import requests
from source_github_v2.streams.branches import BranchesStream
from source_github_v2.streams.commits import CommitsStream
from source_github_v2.streams.pull_requests import PullRequestsStream
from source_github_v2.streams.repositories import RepositoriesStream

TENANT = "T"
SOURCE = "S"

SHARED = {"token": "tok", "tenant_id": TENANT, "source_id": SOURCE}


class FakeResponse(requests.Response):
    """Stub response with canned .json()/.links payloads.

    Subclasses requests.Response because should_retry/backoff_time use an
    isinstance check to distinguish HTTP responses from connection errors.
    """

    def __init__(
        self,
        payload: Any = None,
        status_code: int = 200,
        headers: dict | None = None,
        links: dict | None = None,
        text: str = "",
        url: str = "https://api.github.com/x",
    ):
        super().__init__()
        self._payload = payload
        self.status_code = status_code
        self.headers.update(headers or {})
        self._fake_links = links or {}
        self._content = text.encode("utf-8")
        self.url = url

    @property
    def links(self) -> dict:
        return self._fake_links

    def json(self, **kwargs: Any) -> Any:
        if isinstance(self._payload, Exception):
            raise self._payload
        return self._payload


class FakeRepoParent:
    """RepositoriesStream stub: yields pre-baked child records (owner/name/
    default_branch/pushed_at dicts) without touching disk or HTTP."""

    def __init__(self, records: Iterable[Mapping[str, Any]]):
        self._records = list(records)

    def get_child_records(self):
        yield from self._records


class FakeBranchParent:
    """BranchesStream stub: yields pre-baked branch child records."""

    def __init__(self, records: Iterable[Mapping[str, Any]]):
        self._records = list(records)

    def get_child_records(self):
        yield from self._records


class FakePRParent:
    """PullRequestsStream stub for child streams.

    ``slices`` feeds get_child_slices(); ``embedded`` maps
    (offset, field) -> embedded data dict, mirroring read_embedded_data.
    """

    def __init__(self, slices: Iterable[Mapping[str, Any]] | None = None, embedded: dict | None = None):
        self._slices = list(slices or [])
        self._embedded = embedded or {}

    def get_child_slices(self) -> list:
        return list(self._slices)

    def read_embedded_data(self, offset: int, field: str) -> dict:
        return self._embedded.get((offset, field), {})


class FakeCommitsParent:
    """CommitsStream stub for file_changes: writes a commit-meta TSV."""

    def __init__(self, tmp_path, rows: Iterable[str]):
        self._path = tmp_path / "commits_meta.tsv"
        self._path.write_text("".join(f"{row}\n" for row in rows))

    def get_commit_meta_path(self) -> str:
        return str(self._path)


def graphql_body(
    nodes_path: list[str], nodes: list, page_info: dict | None = None, rate_limit: dict | None = None
) -> dict:
    """Build a GraphQL response body with nodes nested under nodes_path."""
    inner: dict = {"nodes": nodes, "pageInfo": page_info or {"hasNextPage": False, "endCursor": None}}
    for key in reversed(nodes_path):
        inner = {key: inner}
    body: dict = {"data": inner}
    if rate_limit is not None:
        body["data"]["rateLimit"] = rate_limit
    return body


@pytest.fixture
def repositories_stream() -> RepositoriesStream:
    return RepositoriesStream(organizations=["acme"], **SHARED)


@pytest.fixture
def branches_stream(repositories_stream: RepositoriesStream) -> BranchesStream:
    return BranchesStream(parent=repositories_stream, **SHARED)


@pytest.fixture
def commits_stream(branches_stream: BranchesStream) -> CommitsStream:
    return CommitsStream(parent=branches_stream, **SHARED)


@pytest.fixture
def pull_requests_stream(repositories_stream: RepositoriesStream) -> PullRequestsStream:
    return PullRequestsStream(parent=repositories_stream, **SHARED)


def embedded_line(commits=None, reviews=None, comments=None, review_threads=None) -> str:
    """One JSONL line in the format PullRequestsStream writes to disk."""

    def conn(data, thread=False):
        if data is None:
            if thread:
                return {"nodes": [], "threads_has_next_page": False, "threads_end_cursor": None}
            return {"nodes": [], "has_next_page": False, "end_cursor": None}
        return data

    return json.dumps(
        {
            "commits": conn(commits),
            "reviews": conn(reviews),
            "comments": conn(comments),
            "review_threads": conn(review_threads, thread=True),
        },
        separators=(",", ":"),
    )
