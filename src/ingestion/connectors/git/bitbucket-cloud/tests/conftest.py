"""Shared fixtures for source_bitbucket_cloud unit tests.

All tests are offline: HTTP responses are stubbed via FakeResponse and
stream methods are exercised directly (parse_response, request_params,
stream_slices, get_updated_state). No network, no credentials.
"""

from __future__ import annotations

from typing import Any, Iterable, Mapping, Optional

import pytest

from source_bitbucket_cloud.streams.branches import BranchesStream
from source_bitbucket_cloud.streams.commits import CommitsStream
from source_bitbucket_cloud.streams.file_changes import FileChangesStream
from source_bitbucket_cloud.streams.pr_comments import PRCommentsStream
from source_bitbucket_cloud.streams.pr_commits import PRCommitsStream
from source_bitbucket_cloud.streams.pull_requests import PullRequestsStream
from source_bitbucket_cloud.streams.repositories import RepositoriesStream

TENANT = "T"
SOURCE = "S"

SHARED = {"token": "tok", "tenant_id": TENANT, "source_id": SOURCE}


class FakeResponse:
    """Minimal stand-in for requests.Response as consumed by the streams.

    parse_response/_iter_values/next_page_token only touch .json(), .url and
    .status_code.
    """

    def __init__(self, payload: Any, url: str = "https://api.bitbucket.org/2.0/x",
                 status_code: int = 200):
        self._payload = payload
        self.url = url
        self.status_code = status_code

    def json(self) -> Any:
        if isinstance(self._payload, Exception):
            raise self._payload
        return self._payload


class FakeParent:
    """Parent-stream stub: yields pre-baked slices/records without HTTP.

    Mirrors the (stream_slices → read_records) contract every sub-stream in
    this connector drives its parent with.
    """

    def __init__(self, records: Iterable[Mapping[str, Any]],
                 slices: Optional[Iterable[Mapping[str, Any]]] = None):
        self._records = list(records)
        self._slices = list(slices) if slices is not None else [{"dummy": True}]

    def stream_slices(self, **kwargs: Any):
        yield from self._slices

    def read_records(self, **kwargs: Any):
        yield from self._records


@pytest.fixture
def repositories_stream() -> RepositoriesStream:
    return RepositoriesStream(workspaces=["ws"], **SHARED)


@pytest.fixture
def pull_requests_stream(repositories_stream: RepositoriesStream) -> PullRequestsStream:
    return PullRequestsStream(parent=repositories_stream, **SHARED)


@pytest.fixture
def branches_stream(repositories_stream: RepositoriesStream) -> BranchesStream:
    return BranchesStream(parent=repositories_stream, **SHARED)


@pytest.fixture
def commits_stream(branches_stream: BranchesStream) -> CommitsStream:
    return CommitsStream(parent=branches_stream, **SHARED)


@pytest.fixture
def file_changes_stream(commits_stream: CommitsStream) -> FileChangesStream:
    return FileChangesStream(parent=commits_stream, **SHARED)


@pytest.fixture
def pr_comments_stream(pull_requests_stream: PullRequestsStream) -> PRCommentsStream:
    return PRCommentsStream(parent=pull_requests_stream, **SHARED)


@pytest.fixture
def pr_commits_stream(pull_requests_stream: PullRequestsStream) -> PRCommitsStream:
    return PRCommitsStream(parent=pull_requests_stream, **SHARED)
