from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

import pytest
from source_bitbucket_cloud.client import BranchRef, RepositoryRef
from source_bitbucket_cloud.streams.branches import BranchesStream
from source_bitbucket_cloud.streams.commits import CommitsStream
from source_bitbucket_cloud.streams.file_changes import FileChangesStream
from source_bitbucket_cloud.streams.pr_comments import PRCommentsStream
from source_bitbucket_cloud.streams.pr_commits import PRCommitsStream
from source_bitbucket_cloud.streams.pull_requests import PullRequestsStream
from source_bitbucket_cloud.streams.repositories import RepositoriesStream

TENANT = "T"
SOURCE = "S"
SHARED = {"token": "tok", "tenant_id": TENANT, "source_id": SOURCE, "workspaces": ["ws"]}


def repository(slug: str = "repo", uuid: str = "{r-1}", **raw: Any) -> RepositoryRef:
    data = {
        "uuid": uuid,
        "slug": slug,
        "name": slug,
        "full_name": f"ws/{slug}",
        "updated_on": "2026-06-01T00:00:00+00:00",
        **raw,
    }
    return RepositoryRef("ws", "{w-1}", slug, uuid, "main", True, data)


def branch(name: str = "main", sha: str = "a1", **raw: Any) -> BranchRef:
    data = {"name": name, "target": {"hash": sha}, **raw}
    return BranchRef(name, sha, "2026-06-01T00:00:00+00:00", name == "main", data)


class FakeCatalog:
    def __init__(self, repositories: Iterable[RepositoryRef], client: FakeClient | None = None):
        self._repositories = list(repositories)
        self._client = client

    def repositories(self) -> list[RepositoryRef]:
        return self._repositories

    def branches(self, repo: RepositoryRef) -> list[BranchRef]:
        return self._client.branches(repo) if self._client else []


class _FakeResponse:
    def __init__(self, body: Mapping[str, Any]):
        self._body = body

    def json(self) -> Mapping[str, Any]:
        return self._body


class FakeClient:
    def __init__(self):
        self.branch_values: dict[str, list[BranchRef]] = {}
        self.commit_values: list[Mapping[str, Any]] = []
        self.page_values: dict[str, list[Mapping[str, Any]]] = {}
        self.optional_values: dict[str, tuple[bool, list[Mapping[str, Any]]]] = {}
        self.pr_values: list[Mapping[str, Any]] = []
        self.commit_calls: list[tuple[list[str], list[str]]] = []
        self.request_values: dict[str, Mapping[str, Any] | None] = {}

    def branches(self, repo: RepositoryRef) -> list[BranchRef]:
        return self.branch_values.get(repo.uuid, [])

    def request(self, method, path, **kwargs):
        body = self.request_values.get(path)
        if body is None:
            return None
        return _FakeResponse(body)

    def commits_between(self, repo, include, exclude):
        self.commit_calls.append((list(include), list(exclude)))
        return iter(self.commit_values)

    def paginate(self, path, **kwargs):
        if path.endswith("pullrequests"):
            return iter(self.pr_values)
        return iter(self.page_values.get(path, []))

    def paginate_optional(self, path, **kwargs):
        present, values = self.optional_values.get(path, (True, []))
        return present, iter(values)

    def repo_path(self, repo: RepositoryRef, suffix: str) -> str:
        return f"repositories/{repo.workspace}/{repo.slug}/{suffix}"


@pytest.fixture
def repo():
    return repository()


@pytest.fixture
def client():
    return FakeClient()


@pytest.fixture
def stream_args(repo, client):
    return {**SHARED, "client": client, "catalog": FakeCatalog([repo], client)}


@pytest.fixture
def repositories_stream(stream_args):
    return RepositoriesStream(**stream_args)


@pytest.fixture
def branches_stream(stream_args):
    return BranchesStream(**stream_args)


@pytest.fixture
def commits_stream(stream_args):
    return CommitsStream(**stream_args)


@pytest.fixture
def file_changes_stream(stream_args):
    return FileChangesStream(**stream_args)


@pytest.fixture
def pull_requests_stream(stream_args):
    return PullRequestsStream(**stream_args)


@pytest.fixture
def pr_comments_stream(stream_args):
    return PRCommentsStream(**stream_args)


@pytest.fixture
def pr_commits_stream(stream_args):
    return PRCommitsStream(**stream_args)
