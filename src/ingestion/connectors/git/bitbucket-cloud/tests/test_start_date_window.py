from __future__ import annotations

import pytest

from source_bitbucket_cloud.client import BranchRef
from source_bitbucket_cloud.streams.base import BUCKET_COUNT, STATE_VERSION, repo_state_key
from source_bitbucket_cloud.streams.branches import BranchesStream
from source_bitbucket_cloud.streams.commit_branch_reachability import CommitBranchReachabilityStream
from source_bitbucket_cloud.streams.commits import CommitsStream
from source_bitbucket_cloud.streams.file_changes import FileChangesStream
from source_bitbucket_cloud.streams.metric_events import TagsStream
from tests.conftest import SHARED, FakeCatalog, FakeClient, branch, repository

START_DATE = "2026-01-01"
WINDOWED = {**SHARED, "start_date": START_DATE}
RANGE_STREAMS = [CommitsStream, FileChangesStream, CommitBranchReachabilityStream]


class CountingClient(FakeClient):
    def __init__(self):
        super().__init__()
        self.branch_calls = 0

    def branches(self, repo):
        self.branch_calls += 1
        return self.branch_values.get(repo.uuid, [])


def read_all_buckets(stream):
    records = []
    for bucket in range(BUCKET_COUNT):
        records.extend(stream.read_records(None, stream_slice={"bucket_id": bucket}))
    return records


def build(stream_class, client, repo, shared=WINDOWED):
    stream = stream_class(**{**shared, "client": client, "catalog": FakeCatalog([repo], client)})
    stream.state = {}
    return stream


@pytest.mark.parametrize("stream_class", RANGE_STREAMS)
class TestRepositoriesOutsideTheWindow:
    def test_a_repository_untouched_since_start_date_costs_nothing(self, stream_class):
        repo = repository(updated_on="2019-05-01T00:00:00+00:00")
        client = CountingClient()
        client.branch_values[repo.uuid] = [branch("main", "old")]

        stream = build(stream_class, client, repo)
        records = read_all_buckets(stream)

        assert records == []
        assert client.branch_calls == 0, "no push since start_date means nothing to read"
        assert stream.state["repositories"] == {}, "a gated repository must not grow the state"

    def test_a_repository_pushed_inside_the_window_syncs(self, stream_class):
        repo = repository(updated_on="2026-06-01T00:00:00+00:00")
        client = CountingClient()
        client.branch_values[repo.uuid] = [branch("main", "head", **{"target": {"date": "2026-06-01T00:00:00+00:00"}})]
        client.commit_values = [{"hash": "head", "date": "2026-06-01T00:00:00+00:00"}]

        stream = build(stream_class, client, repo)
        read_all_buckets(stream)

        assert client.branch_calls > 0

    def test_a_repository_without_updated_on_is_never_gated(self, stream_class):
        repo = repository()
        repo.raw.pop("updated_on")
        client = CountingClient()
        client.branch_values[repo.uuid] = [branch("main", "head")]
        client.commit_values = [{"hash": "head", "date": "2026-06-01T00:00:00+00:00"}]

        stream = build(stream_class, client, repo)
        read_all_buckets(stream)

        assert client.branch_calls > 0, "an unknown push date must be read, not assumed stale"

    def test_no_start_date_gates_nothing(self, stream_class):
        repo = repository(updated_on="2019-05-01T00:00:00+00:00")
        client = CountingClient()
        client.branch_values[repo.uuid] = [branch("main", "old")]
        client.commit_values = [{"hash": "old", "date": "2019-05-01T00:00:00+00:00"}]

        stream = build(stream_class, client, repo, shared=SHARED)
        read_all_buckets(stream)

        assert client.branch_calls > 0


class TestBranchesAreCurrentStateNotHistory:
    """Branches exist now regardless of when they were last pushed to, so the
    window must not empty a dormant repository's branch snapshot."""

    def dormant(self):
        repo = repository(updated_on="2019-05-01T00:00:00+00:00")
        client = CountingClient()
        client.branch_values[repo.uuid] = [branch("main", "old")]
        return repo, client

    def test_a_dormant_repository_still_reports_its_branches(self):
        repo, client = self.dormant()

        records = read_all_buckets(build(BranchesStream, client, repo))

        assert [r["name"] for r in records if r["record_type"] == "item"] == ["main"]
        marker = next(r for r in records if r["record_type"] == "snapshot_complete")
        assert marker["snapshot_available"] is True
        assert marker["snapshot_item_count"] == 1

    def test_and_costs_one_listing_ever(self):
        repo, client = self.dormant()
        first = build(BranchesStream, client, repo)
        read_all_buckets(first)

        second = build(BranchesStream, client, repo)
        second.state = first.state
        read_all_buckets(second)

        assert client.branch_calls == 1, "the idle gate, not start_date, is what keeps a dormant repository cheap"


def dated_branch(name: str, sha: str, target_date: str | None):
    return BranchRef(name=name, head_sha=sha, target_date=target_date, is_default=name == "main")


class TestColdRepositoriesSkipStaleBranches:
    """A repository inside the window can still carry branches parked years
    ago; ranging those on a first read pages their whole history for commits
    the date filter then discards."""

    def make(self, stream_class):
        repo = repository(updated_on="2026-06-01T00:00:00+00:00")
        client = FakeClient()
        client.branch_values[repo.uuid] = [
            dated_branch("main", "fresh", "2026-06-01T00:00:00+00:00"),
            dated_branch("ancient", "stale", "2015-02-01T00:00:00+00:00"),
        ]
        client.commit_values = [{"hash": "fresh", "date": "2026-06-01T00:00:00+00:00"}]
        return build(stream_class, client, repo), client, repo

    @pytest.mark.parametrize("stream_class", [CommitsStream, FileChangesStream])
    def test_first_read_ranges_only_in_window_heads(self, stream_class):
        stream, client, _ = self.make(stream_class)

        read_all_buckets(stream)

        assert client.commit_calls == [(["fresh"], [])]

    @pytest.mark.parametrize("stream_class", [CommitsStream, FileChangesStream])
    def test_stale_heads_are_still_stored_so_a_later_push_diffs(self, stream_class):
        stream, _, repo = self.make(stream_class)

        read_all_buckets(stream)

        assert stream.state["repositories"][repo_state_key(repo)]["head_shas"] == ["fresh", "stale"]

    def test_reachability_skips_the_stale_branch_only(self):
        stream, client, _ = self.make(CommitBranchReachabilityStream)

        records = read_all_buckets(stream)

        assert {r["branch_name"] for r in records if r["record_type"] == "item"} == {"main"}
        assert client.commit_calls == [(["fresh"], [])]

    @pytest.mark.parametrize("stream_class", [CommitsStream, FileChangesStream])
    def test_a_known_repository_ranges_every_head(self, stream_class):
        stream, client, repo = self.make(stream_class)
        stream.state = {
            "version": STATE_VERSION,
            "bucket_count": BUCKET_COUNT,
            "repositories": {repo_state_key(repo): {"head_shas": ["older"], "repo_updated_on": "2026-05-01"}},
        }

        read_all_buckets(stream)

        assert client.commit_calls == [(["fresh", "stale"], ["older"])], (
            "once heads are known the exclude set bounds the read, so nothing needs skipping"
        )

    def test_a_branch_without_a_target_date_is_read(self):
        repo = repository(updated_on="2026-06-01T00:00:00+00:00")
        client = FakeClient()
        client.branch_values[repo.uuid] = [dated_branch("main", "undated", None)]
        client.commit_values = [{"hash": "undated", "date": "2026-06-01T00:00:00+00:00"}]

        read_all_buckets(build(CommitsStream, client, repo))

        assert client.commit_calls == [(["undated"], [])]


class TestTagsHonourTheWindow:
    def tag(self, name: str, tagged_at: str | None, commit_date: str = "2015-01-01T00:00:00+00:00"):
        """An annotated tag carries its own date; `target` is the commit it
        points at, which can be far older than the tag."""
        record = {"name": name, "target": {"hash": f"{name}-sha", "date": commit_date}}
        if tagged_at:
            record["date"] = tagged_at
        return record

    def read(self, tags):
        repo = repository(updated_on="2026-06-01T00:00:00+00:00")
        client = FakeClient()
        client.optional_values[client.repo_path(repo, "refs/tags")] = (True, tags)
        stream = TagsStream(**{**WINDOWED, "client": client, "catalog": FakeCatalog([repo], client)})
        return read_all_buckets(stream)

    def test_tags_older_than_start_date_are_dropped(self):
        records = self.read([self.tag("v1", "2015-01-01T00:00:00+00:00"), self.tag("v9", "2026-06-01T00:00:00+00:00")])

        assert [r["name"] for r in records if r["record_type"] == "item"] == ["v9"]

    def test_the_marker_counts_only_what_was_emitted(self):
        records = self.read([self.tag("v1", "2015-01-01T00:00:00+00:00"), self.tag("v9", "2026-06-01T00:00:00+00:00")])

        marker = next(r for r in records if r["record_type"] == "snapshot_complete")
        assert marker["snapshot_item_count"] == 1, "completeness must match the filtered snapshot"
        assert marker["snapshot_available"] is True

    def test_an_undated_tag_is_kept(self):
        records = self.read([self.tag("v1", None)])

        assert [r["name"] for r in records if r["record_type"] == "item"] == ["v1"]

    def test_a_new_tag_on_an_old_commit_is_kept(self):
        """Judging a tag by its commit would drop every release cut against
        history — the tag is the event, not the commit it names."""
        records = self.read([self.tag("v9", "2026-06-01T00:00:00+00:00", commit_date="2015-01-01T00:00:00+00:00")])

        assert [r["name"] for r in records if r["record_type"] == "item"] == ["v9"]
