from airbyte_cdk.models import SyncMode
from conftest import SHARED, FakeCatalog, FakeClient, branch
from source_bitbucket_cloud.client import BitbucketApiError
from source_bitbucket_cloud.streams.base import repository_bucket
from source_bitbucket_cloud.streams.commit_branch_reachability import CommitBranchReachabilityStream


def reachability_stream(repo, client):
    return CommitBranchReachabilityStream(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})


def with_prior_heads(stream, repo, heads):
    stream.state = {"version": 2, "bucket_count": 8, "repositories": {repo.uuid: {"heads": heads}}}


def read_reachability(stream, repo):
    return list(stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": repository_bucket(repo.uuid)}))


def test_branch_snapshot_reads_provider_and_marks_default(branches_stream, client, repo):
    client.branch_values[repo.uuid] = [branch("main", "a"), branch("release", "b")]
    records = list(
        branches_stream.read_records(SyncMode.full_refresh, stream_slice={"bucket_id": repository_bucket(repo.uuid)})
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
        branches_stream.read_records(SyncMode.full_refresh, stream_slice={"bucket_id": repository_bucket(repo.uuid)})
    )
    assert records[-1]["snapshot_item_count"] == 1


def test_reachability_emits_commits_for_every_changed_branch(stream_args, client, repo):
    stream = CommitBranchReachabilityStream(**stream_args)
    client.branch_values[repo.uuid] = [branch("main", "m1"), branch("release", "r1")]
    client.commit_values = [{"hash": "c1", "date": "2026-06-01"}]
    records = list(stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": repository_bucket(repo.uuid)}))
    assert {record["branch_name"] for record in records} == {"main", "release"}
    assert all(record["reachability_action"] == "added" for record in records)
    assert stream.state["repositories"][repo.uuid]["heads"] == {"main": "m1", "release": "r1"}


def test_reachability_records_deleted_branch(stream_args, client, repo):
    stream = CommitBranchReachabilityStream(**stream_args)
    stream.state = {"version": 2, "bucket_count": 8, "repositories": {repo.uuid: {"heads": {"release": "old"}}}}
    client.branch_values[repo.uuid] = []
    records = list(stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": repository_bucket(repo.uuid)}))
    assert records[0]["branch_name"] == "release"
    assert records[0]["reachability_action"] == "branch_deleted"
    assert records[0]["commit_sha"] is None


def test_reachability_moved_branch_emits_added_and_removed(repo):
    client = FakeClient()
    client.branch_values[repo.uuid] = [branch("main", "new")]
    client.commit_values = [{"hash": "c1", "date": "2026-06-01"}]
    stream = reachability_stream(repo, client)
    with_prior_heads(stream, repo, {"main": "old"})

    records = read_reachability(stream, repo)

    assert {record["reachability_action"] for record in records} == {"added", "removed"}
    assert stream.state["repositories"][repo.uuid]["heads"] == {"main": "new"}


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
