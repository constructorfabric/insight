from __future__ import annotations

import json

import pytest

from source_bitbucket_cloud.client import BitbucketApiError
from source_bitbucket_cloud.streams.base import BUCKET_COUNT, repo_state_key
from source_bitbucket_cloud.streams.commit_branch_reachability import CommitBranchReachabilityStream
from source_bitbucket_cloud.streams.commits import CommitsStream
from source_bitbucket_cloud.streams.git_ranges import RANGE_REPAIR_ATTEMPTS
from tests.conftest import SHARED, FakeCatalog, FakeClient, branch, repository

DATE = "2026-06-01T00:00:00+00:00"


def commit_not_found(*shas: str) -> BitbucketApiError:
    body = json.dumps({"type": "error", "error": {"message": "Commit not found", "data": {"shas": list(shas)}}})
    return BitbucketApiError(404, "https://api.bitbucket.org/2.0/x/commits", body)


class UnresolvableClient(FakeClient):
    """Rejects any range that still references a sha the API cannot resolve."""

    def __init__(self, unresolvable: set[str]):
        super().__init__()
        self.unresolvable = unresolvable

    def commits_between(self, repo, include, exclude):
        self.commit_calls.append((list(include), list(exclude)))
        referenced = self.unresolvable.intersection(set(include) | set(exclude))
        if referenced:
            raise commit_not_found(*sorted(referenced))
        return iter([{"hash": sha, "date": DATE} for sha in include])


def read_all_buckets(stream):
    records = []
    for bucket in range(BUCKET_COUNT):
        records.extend(stream.read_records(None, stream_slice={"bucket_id": bucket}))
    return records


def build(cls, client, repo):
    stream = cls(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})
    stream.state = {}
    return stream


class TestMissingShasArePruned:
    def test_readable_heads_still_sync(self, repo):
        client = UnresolvableClient({"ghost"})
        client.branch_values[repo.uuid] = [branch("main", "alive"), branch("old", "ghost")]

        records = read_all_buckets(build(CommitsStream, client, repo))

        assert [r["hash"] for r in records] == ["alive"], (
            "one unresolvable head must not cost the repository its other branches"
        )
        assert client.commit_calls[-1] == (["alive"], [])

    def test_error_names_the_shas_to_drop(self):
        assert commit_not_found("a", "b").missing_shas == frozenset({"a", "b"})

    @pytest.mark.parametrize("body", ["", "{}", json.dumps({"error": {"data": {"shas": "nope"}}})])
    def test_absent_sha_lists_read_as_empty(self, body):
        assert BitbucketApiError(404, "u", body).missing_shas == frozenset()

    def test_repository_with_no_resolvable_head_yields_nothing(self, repo):
        client = UnresolvableClient({"ghost"})
        client.branch_values[repo.uuid] = [branch("old", "ghost")]

        stream = build(CommitsStream, client, repo)
        records = read_all_buckets(stream)

        assert records == []
        assert stream._failed_repositories == [], "an unresolvable head is not a sync failure"
        assert stream.state["repositories"][repo_state_key(repo)]["head_shas"] == []

    def test_an_unread_head_is_not_checkpointed(self, repo):
        client = UnresolvableClient({"ghost"})
        client.branch_values[repo.uuid] = [branch("main", "alive"), branch("old", "ghost")]

        stream = build(CommitsStream, client, repo)
        read_all_buckets(stream)

        stored = stream.state["repositories"][repo_state_key(repo)]
        assert stored["head_shas"] == ["alive"], "recording a head we could not read would claim it was synced"
        assert stored["repo_updated_on"] == "", "the cursor must stay open so the head is tried again"

    def test_a_head_that_resolves_later_is_read_without_a_new_push(self, repo):
        """The 404 may be transient. Nothing else moves in the repository — no
        push, so no new updated_on — and the head must still be picked up."""
        client = UnresolvableClient({"ghost"})
        client.branch_values[repo.uuid] = [branch("main", "alive"), branch("old", "ghost")]
        first = build(CommitsStream, client, repo)
        read_all_buckets(first)

        client.unresolvable.clear()
        client.commit_calls.clear()
        second = build(CommitsStream, client, repo)
        second.state = first.state
        records = read_all_buckets(second)

        assert client.commit_calls, "the idle gate must not close over an unread head"
        assert "ghost" in [r["hash"] for r in records]
        assert second.state["repositories"][repo_state_key(repo)]["head_shas"] == ["alive", "ghost"]

    def test_stale_excludes_are_still_dropped_when_no_shas_are_named(self, repo):
        class BareNotFound(FakeClient):
            def commits_between(self, repo, include, exclude):
                self.commit_calls.append((list(include), list(exclude)))
                if exclude:
                    raise BitbucketApiError(404, "u", "gone")
                return iter([{"hash": "new", "date": DATE}])

        client = BareNotFound()
        client.branch_values[repo.uuid] = [branch("main", "new")]
        stream = build(CommitsStream, client, repo)
        stream.state = {
            "version": 3,
            "bucket_count": 8,
            "repositories": {repo_state_key(repo): {"head_shas": ["old"], "repo_updated_on": "stale"}},
        }

        records = read_all_buckets(stream)

        assert [r["hash"] for r in records] == ["new"]
        assert client.commit_calls == [(["new"], ["old"]), (["new"], [])]

    def test_repair_keeps_going_while_it_is_getting_somewhere(self, repo):
        """The API names only the shas it noticed, so a repository with many
        dead heads needs many rounds. Stopping part-way would leave it failing
        the same way on every future sync."""
        heads = 20

        class OneAtATime(FakeClient):
            """Names a single dead head per answer."""

            def commits_between(self, repo, include, exclude):
                self.commit_calls.append((list(include), list(exclude)))
                raise commit_not_found(sorted(include)[0])

        client = OneAtATime()
        client.branch_values[repo.uuid] = [branch(f"b{index}", f"sha{index:02d}") for index in range(heads)]
        stream = build(CommitsStream, client, repo)

        records = read_all_buckets(stream)

        assert records == []
        assert stream._failed_repositories == [], "pruning to nothing is not a failure"
        assert not stream._catalog.is_inaccessible(repo), "nor a denial: the listing was readable"
        assert len(client.commit_calls) == heads, (
            f"every dead head must be pruned; stopped after {len(client.commit_calls)} of {heads}"
        )
        assert len(client.commit_calls) > RANGE_REPAIR_ATTEMPTS, "and past the warning threshold"

    def test_repair_still_terminates_when_nothing_can_be_pruned(self, repo):
        class NamesSomethingElse(FakeClient):
            def commits_between(self, repo, include, exclude):
                self.commit_calls.append((list(include), list(exclude)))
                raise commit_not_found("a-sha-not-in-this-range")

        client = NamesSomethingElse()
        client.branch_values[repo.uuid] = [branch("main", "head")]
        stream = build(CommitsStream, client, repo)
        stream.state = {
            "version": 3,
            "bucket_count": 8,
            "repositories": {repo_state_key(repo): {"head_shas": ["old"], "repo_updated_on": "stale"}},
        }

        read_all_buckets(stream)

        assert len(client.commit_calls) == 2, "clear the excludes once, then give up"
        assert stream._catalog.is_inaccessible(repo)

    def test_unnamed_404_without_excludes_is_a_denial(self, repo):
        class BareNotFound(FakeClient):
            def commits_between(self, repo, include, exclude):
                self.commit_calls.append((list(include), list(exclude)))
                raise BitbucketApiError(404, "u", "gone")

        client = BareNotFound()
        client.branch_values[repo.uuid] = [branch("main", "head")]
        stream = build(CommitsStream, client, repo)

        read_all_buckets(stream)

        assert stream._failed_repositories == []
        assert stream._catalog.is_inaccessible(repo)


class TestReachabilitySkipsVanishedHeads:
    def test_vanished_head_skips_its_branch_only(self, repo):
        client = UnresolvableClient({"ghost"})
        client.branch_values[repo.uuid] = [branch("main", "alive"), branch("old", "ghost")]

        stream = build(CommitBranchReachabilityStream, client, repo)
        records = read_all_buckets(stream)

        assert stream._failed_repositories == []
        branches_seen = {r["branch_name"] for r in records if r["record_type"] == "item"}
        assert branches_seen == {"main"}

    def test_the_skipped_branch_is_not_checkpointed(self, repo):
        client = UnresolvableClient({"ghost"})
        client.branch_values[repo.uuid] = [branch("main", "alive"), branch("old", "ghost")]

        stream = build(CommitBranchReachabilityStream, client, repo)
        read_all_buckets(stream)

        stored = stream.state["repositories"][repo_state_key(repo)]
        assert stored["heads"] == {"main": "alive"}
        assert stored["repo_updated_on"] == ""
