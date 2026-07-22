from __future__ import annotations

from dataclasses import replace

import pytest

from source_bitbucket_cloud.client import BitbucketApiError, BitbucketClient, RepositoryCatalog
from source_bitbucket_cloud.streams.base import BUCKET_COUNT
from source_bitbucket_cloud.streams.commits import CommitsStream
from source_bitbucket_cloud.streams.metric_events import IssuesStream
from tests.conftest import SHARED, FakeCatalog, FakeClient, branch, repository


def read_all_buckets(stream):
    records, error = [], None
    for bucket in range(BUCKET_COUNT):
        try:
            records.extend(stream.read_records(None, stream_slice={"bucket_id": bucket}))
        except RuntimeError as exc:
            error = exc
    return records, error


class BrokenBranchesClient(FakeClient):
    def __init__(self, broken_uuid):
        super().__init__()
        self.broken_uuid = broken_uuid

    def branches(self, repo):
        if repo.uuid == self.broken_uuid:
            raise RuntimeError("boom")
        return super().branches(repo)


class TestRepositoryQuarantine:
    def make_stream(self, client, repos):
        return CommitsStream(**{**SHARED, "client": client, "catalog": FakeCatalog(repos, client)})

    def test_one_broken_repo_does_not_block_others(self):
        good = repository(slug="good", uuid="{good}")
        bad = repository(slug="bad", uuid="{bad}")
        client = BrokenBranchesClient("{bad}")
        client.branch_values["{good}"] = [branch("main", "a1")]
        client.commit_values = [{"hash": "a1", "date": "2026-06-01T00:00:00+00:00"}]
        stream = self.make_stream(client, [good, bad])
        stream.state = {}

        records, error = read_all_buckets(stream)

        assert [record["hash"] for record in records] == ["a1"]
        assert error is not None and "bad" in str(error)

    def test_failed_repo_state_not_advanced(self):
        bad = repository(slug="bad", uuid="{bad}")
        client = BrokenBranchesClient("{bad}")
        stream = self.make_stream(client, [bad])
        stream.state = {}

        _, error = read_all_buckets(stream)

        assert error is not None
        assert stream.state["repositories"] == {}

    def test_no_failures_no_error(self):
        good = repository(slug="good", uuid="{good}")
        client = FakeClient()
        client.branch_values["{good}"] = [branch("main", "a1")]
        client.commit_values = [{"hash": "a1", "date": "2026-06-01T00:00:00+00:00"}]
        stream = self.make_stream(client, [good])
        stream.state = {}

        records, error = read_all_buckets(stream)

        assert error is None
        assert len(records) == 1
        assert stream.state["repositories"]["{good}"] == {"head_shas": ["a1"]}


class TestIssuesDisabledRepos:
    def test_disabled_issue_tracker_leaves_no_watermark(self):
        no_issues_repo = replace(repository(slug="noissues", uuid="{ni}"), has_issues=False)
        client = FakeClient()
        stream = IssuesStream(**{**SHARED, "client": client, "catalog": FakeCatalog([no_issues_repo], client)})
        stream.state = {}

        records, error = read_all_buckets(stream)

        assert records == [] and error is None
        assert stream.state["repositories"].get("{ni}", {}).get("updated_on") is None


class TestBranchListingMemoized:
    def test_catalog_lists_each_repository_once(self):
        calls = []

        class CountingClient(FakeClient):
            def branches(self, repo):
                calls.append(repo.uuid)
                return [branch("main", "a1")]

        repo = repository()
        client = CountingClient()
        catalog = RepositoryCatalog(client, ["ws"], True)
        catalog._repositories = [repo]

        first = catalog.branches(repo)
        second = catalog.branches(repo)

        assert first == second
        assert calls == [repo.uuid]


class TestClientHardening:
    def make_client(self):
        return BitbucketClient("tok")

    def test_rejects_foreign_next_url(self):
        client = self.make_client()
        with pytest.raises(RuntimeError, match="Refusing to follow"):
            client._url("https://evil.example.com/2.0/repositories")

    def test_accepts_own_base_url(self):
        client = self.make_client()
        url = client._url("https://api.bitbucket.org/2.0/repositories/ws?page=2")
        assert url.startswith("https://api.bitbucket.org/2.0/")

    def test_relative_path_joined(self):
        client = self.make_client()
        assert client._url("repositories/ws") == "https://api.bitbucket.org/2.0/repositories/ws"

    def test_invalid_json_raises_clear_error(self):
        client = self.make_client()

        class FakeResponse:
            url = "https://api.bitbucket.org/2.0/x"

            def json(self):
                raise ValueError("bad json")

        with pytest.raises(RuntimeError, match="invalid JSON"):
            client._json(FakeResponse())

    def test_commits_between_requests_large_pages(self):
        client = self.make_client()
        seen = {}

        def fake_paginate(path, *, params=None, method="GET", data=None, **kwargs):
            seen.update({"params": params, "method": method, "data": data})
            return iter(())

        client.paginate = fake_paginate
        list(client.commits_between(repository(), ["new1", "new2"], ["old1"]))
        assert seen["method"] == "POST"
        assert seen["params"] == {"pagelen": "100"}
        assert ("include", "new1") in seen["data"] and ("exclude", "old1") in seen["data"]


class TestNewCommits404Recovery:
    def make_stream(self, client, repo):
        return CommitsStream(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})

    def test_404_with_previous_heads_retries_without_excludes(self):
        repo = repository()

        class Client(FakeClient):
            def commits_between(self, repo, include, exclude):
                self.commit_calls.append((list(include), list(exclude)))
                if exclude:
                    raise BitbucketApiError(404, "https://api.bitbucket.org/2.0/x", "gone")
                return iter([{"hash": "c1", "date": "2026-06-01"}])

        client = Client()
        stream = self.make_stream(client, repo)

        result = list(stream.new_commits(repo, ["new"], ["old"]))

        assert [commit["hash"] for commit in result] == ["c1"]
        assert client.commit_calls == [(["new"], ["old"]), (["new"], [])]

    def test_non_404_propagates(self):
        repo = repository()

        class Client(FakeClient):
            def commits_between(self, repo, include, exclude):
                raise BitbucketApiError(500, "https://api.bitbucket.org/2.0/x", "boom")

        stream = self.make_stream(Client(), repo)
        with pytest.raises(BitbucketApiError):
            list(stream.new_commits(repo, ["new"], ["old"]))

    def test_404_without_previous_heads_propagates(self):
        repo = repository()

        class Client(FakeClient):
            def commits_between(self, repo, include, exclude):
                raise BitbucketApiError(404, "https://api.bitbucket.org/2.0/x", "gone")

        stream = self.make_stream(Client(), repo)
        with pytest.raises(BitbucketApiError):
            list(stream.new_commits(repo, ["new"], []))


class TestStartDateValidation:
    def test_invalid_start_date_message(self):
        client = FakeClient()
        with pytest.raises(ValueError, match="bitbucket_start_date"):
            CommitsStream(
                **{**SHARED, "start_date": "junk", "client": client, "catalog": FakeCatalog([], client)}
            )
