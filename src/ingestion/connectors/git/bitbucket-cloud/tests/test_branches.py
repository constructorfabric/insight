from airbyte_cdk.models import SyncMode
from conftest import SHARED, FakeCatalog, FakeClient, branch
from source_bitbucket_cloud.client import BitbucketApiError
from source_bitbucket_cloud.streams.base import repo_state_key, repository_bucket
from source_bitbucket_cloud.streams.commit_branch_reachability import (
    RANGE_PREFETCH,
    CommitBranchReachabilityStream,
)


def reachability_stream(repo, client):
    return CommitBranchReachabilityStream(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})


def with_prior_heads(stream, repo, heads):
    stream.state = {"version": 3, "bucket_count": 8, "repositories": {repo_state_key(repo): {"heads": heads}}}


def read_reachability(stream, repo):
    return list(stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": repository_bucket(repo_state_key(repo))}))


def test_branch_snapshot_reads_provider_and_marks_default(branches_stream, client, repo):
    client.branch_values[repo.uuid] = [branch("main", "a"), branch("release", "b")]
    records = list(
        branches_stream.read_records(SyncMode.full_refresh, stream_slice={"bucket_id": repository_bucket(repo_state_key(repo))})
    )
    items = records[:-1]
    assert [item["name"] for item in items] == ["main", "release"]
    assert items[0]["is_default"] is True
    assert items[1]["is_default"] is False
    assert records[-1]["snapshot_item_count"] == 2
    assert set(items[0]) <= set(branches_stream.get_json_schema()["properties"])


def test_branch_snapshot_counts_duplicate_entities_once(branches_stream, client, repo):
    client.branch_values[repo.uuid] = [branch("main", "a"), branch("main", "a")]
    records = list(
        branches_stream.read_records(SyncMode.full_refresh, stream_slice={"bucket_id": repository_bucket(repo_state_key(repo))})
    )
    assert records[-1]["snapshot_item_count"] == 1


def test_reachability_emits_commits_for_every_changed_branch(stream_args, client, repo):
    stream = CommitBranchReachabilityStream(**stream_args)
    client.branch_values[repo.uuid] = [branch("main", "m1"), branch("release", "r1")]
    client.commit_values = [{"hash": "c1", "date": "2026-06-01"}]
    records = list(stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": repository_bucket(repo_state_key(repo))}))
    assert {record["branch_name"] for record in records} == {"main", "release"}
    assert all(record["reachability_action"] == "added" for record in records)
    assert stream.state["repositories"][repo_state_key(repo)]["heads"] == {"main": "m1", "release": "r1"}


def test_reachability_records_deleted_branch(stream_args, client, repo):
    stream = CommitBranchReachabilityStream(**stream_args)
    stream.state = {
        "version": 3,
        "bucket_count": 8,
        "repositories": {repo_state_key(repo): {"heads": {"main": "m1", "release": "old"}}},
    }
    client.branch_values[repo.uuid] = [branch("main", "m1")]
    records = list(stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": repository_bucket(repo_state_key(repo))}))
    deleted = [record for record in records if record["reachability_action"] == "branch_deleted"]
    assert [record["branch_name"] for record in deleted] == ["release"]
    assert deleted[0]["commit_sha"] is None


def test_reachability_does_not_delete_every_branch_over_one_empty_listing(stream_args, client, repo):
    """A listing that returns nothing would mark the whole repository deleted,
    and a later listing that finds the branches again emits no correction."""
    stream = CommitBranchReachabilityStream(**stream_args)
    prior = {"heads": {"main": "m1", "release": "old"}, "repo_updated_on": "earlier"}
    stream.state = {"version": 3, "bucket_count": 8, "repositories": {repo_state_key(repo): prior}}
    client.branch_values[repo.uuid] = []

    records = list(
        stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": repository_bucket(repo_state_key(repo))})
    )

    assert records == []
    assert stream.state["repositories"][repo_state_key(repo)] == prior, "nothing read, nothing advanced"


def test_reachability_moved_branch_emits_added_and_removed(repo):
    client = FakeClient()
    client.branch_values[repo.uuid] = [branch("main", "new")]
    client.commit_values = [{"hash": "c1", "date": "2026-06-01"}]
    stream = reachability_stream(repo, client)
    with_prior_heads(stream, repo, {"main": "old"})

    records = read_reachability(stream, repo)

    assert {record["reachability_action"] for record in records} == {"added", "removed"}
    assert stream.state["repositories"][repo_state_key(repo)]["heads"] == {"main": "new"}


class _Raise404WhenExcluding(FakeClient):
    def commits_between(self, repo, include, exclude):
        self.commit_calls.append((list(include), list(exclude)))
        if exclude:
            raise BitbucketApiError(404, "https://api.bitbucket.org/2.0/x", "no such commit")
        return iter(self.commit_values)


def test_reachability_404_resets_added_and_marks_removal_unavailable(repo):
    client = _Raise404WhenExcluding()
    client.branch_values[repo.uuid] = [branch("main", "new")]
    client.commit_values = [{"hash": "c1", "date": "2026-06-01"}]
    stream = reachability_stream(repo, client)
    with_prior_heads(stream, repo, {"main": "old"})

    records = read_reachability(stream, repo)
    actions = {record["reachability_action"] for record in records}

    assert "reset" in actions
    assert "removal_unavailable" in actions
    assert "added" not in actions


class _YieldThenFailWhenExcluding(FakeClient):
    def commits_between(self, repo, include, exclude):
        self.commit_calls.append((list(include), list(exclude)))
        if exclude:
            def partial():
                yield {"hash": "partial", "date": "2026-06-01"}
                raise BitbucketApiError(404, "https://api.bitbucket.org/2.0/x", "page gone")

            return partial()
        return iter([{"hash": "full", "date": "2026-06-01"}])


def test_reachability_404_after_partial_page_does_not_re_emit(repo):
    client = _YieldThenFailWhenExcluding()
    client.branch_values[repo.uuid] = [branch("main", "new")]
    stream = reachability_stream(repo, client)
    with_prior_heads(stream, repo, {"main": "old"})

    records = read_reachability(stream, repo)
    emitted = {(record["reachability_action"], record["commit_sha"]) for record in records}

    assert ("added", "partial") not in emitted
    assert ("reset", "full") in emitted


class _CountingRange(FakeClient):
    """Counts how many commits the caller has pulled out of the range."""

    def __init__(self, size):
        super().__init__()
        self.size = size
        self.pulled = 0

    def commits_between(self, repo, include, exclude):
        self.commit_calls.append((list(include), list(exclude)))

        def pages():
            for index in range(self.size):
                self.pulled += 1
                yield {"hash": f"c{index}", "date": "2026-06-01"}

        return pages()


def test_reachability_does_not_hold_a_whole_history_in_memory(repo):
    """A first read of a branch spans its entire history and several
    repositories are read at once, so the range may not be materialised."""
    history = RANGE_PREFETCH * 3
    client = _CountingRange(history)
    client.branch_values[repo.uuid] = [branch("main", "new")]
    stream = reachability_stream(repo, client)
    stream.state = {}

    records = stream.read_records(
        SyncMode.incremental, stream_slice={"bucket_id": repository_bucket(repo_state_key(repo))}
    )
    first = next(records)

    assert first["commit_sha"] == "c0"
    assert client.pulled <= RANGE_PREFETCH, f"buffered {client.pulled} of {history} before emitting anything"
    assert len(list(records)) == history - 1, "and the rest of the range still arrives"
