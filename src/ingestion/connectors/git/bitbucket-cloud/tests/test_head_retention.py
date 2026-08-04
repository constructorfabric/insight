from __future__ import annotations

import pytest

from source_bitbucket_cloud.streams.base import BUCKET_COUNT, STATE_VERSION, repo_state_key
from source_bitbucket_cloud.streams.branches import BranchesStream
from source_bitbucket_cloud.streams.commit_branch_reachability import CommitBranchReachabilityStream
from source_bitbucket_cloud.streams.commits import CommitsStream
from source_bitbucket_cloud.streams.file_changes import FileChangesStream
from tests.conftest import SHARED, FakeCatalog, FakeClient, branch, repository

DATE = "2026-06-01T00:00:00+00:00"
HEAD_FIELD = {
    CommitsStream: "head_shas",
    FileChangesStream: "head_shas",
    CommitBranchReachabilityStream: "heads",
}


def read_all_buckets(stream):
    records = []
    for bucket in range(BUCKET_COUNT):
        records.extend(stream.read_records(None, stream_slice={"bucket_id": bucket}))
    return records


def synced_state(repo, field, value, updated_on):
    return {
        "version": STATE_VERSION,
        "bucket_count": BUCKET_COUNT,
        "repositories": {repo_state_key(repo): {field: value, "repo_updated_on": updated_on}},
    }


@pytest.mark.parametrize("stream_class", list(HEAD_FIELD))
class TestEmptyListingDoesNotForgetHeads:
    def build(self, stream_class, client, repo):
        return stream_class(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})

    def known(self, stream_class):
        return ["known"] if HEAD_FIELD[stream_class] == "head_shas" else {"main": "known"}

    def test_heads_survive_a_listing_that_returns_nothing(self, stream_class, repo):
        field = HEAD_FIELD[stream_class]
        client = FakeClient()
        client.branch_values[repo.uuid] = []
        stream = self.build(stream_class, client, repo)
        stream.state = synced_state(repo, field, self.known(stream_class), "older")

        read_all_buckets(stream)

        stored = stream.state["repositories"][repo_state_key(repo)]
        assert stored[field] == self.known(stream_class), (
            "an empty listing must not cost the exclude set — the next range would re-read all history"
        )
        assert stored["repo_updated_on"] == "older", (
            "advancing the cursor over an empty listing would gate away whatever the push carried"
        )

    def test_the_listing_is_retried_even_if_nothing_is_pushed_after_it(self, stream_class, repo):
        """The empty answer may have been the API's, not the repository's. The
        next pass must look again rather than trust an idle gate that was
        closed by a read which saw nothing."""
        field = HEAD_FIELD[stream_class]
        client = FakeClient()
        client.branch_values[repo.uuid] = []
        stream = self.build(stream_class, client, repo)
        stream.state = synced_state(repo, field, self.known(stream_class), "older")
        read_all_buckets(stream)

        client.branch_values[repo.uuid] = [branch("main", "fresh")]
        client.commit_values = [{"hash": "fresh", "date": DATE}]
        retried = self.build(stream_class, client, repo)
        retried.state = stream.state
        client.commit_calls.clear()

        read_all_buckets(retried)

        assert client.commit_calls, "the repository must be looked at again"
        assert all(excludes for _, excludes in client.commit_calls), "and diffed against the retained head"

    def test_a_reappearing_branch_is_diffed_not_re_read(self, stream_class, repo):
        field = HEAD_FIELD[stream_class]
        client = FakeClient()
        client.branch_values[repo.uuid] = []
        stream = self.build(stream_class, client, repo)
        stream.state = synced_state(repo, field, self.known(stream_class), "older")
        read_all_buckets(stream)

        pushed = repository(updated_on="2026-07-01T00:00:00+00:00")
        client.branch_values[pushed.uuid] = [branch("main", "fresh")]
        client.commit_values = [{"hash": "fresh", "date": DATE}]
        revived = self.build(stream_class, client, pushed)
        revived.state = stream.state
        client.commit_calls.clear()
        read_all_buckets(revived)

        assert client.commit_calls, "the revived branch must be fetched"
        assert all(excludes for _, excludes in client.commit_calls), (
            "every range must carry the retained head as an exclude"
        )

    def test_a_populated_listing_still_replaces_the_stored_heads(self, stream_class, repo):
        field = HEAD_FIELD[stream_class]
        client = FakeClient()
        client.branch_values[repo.uuid] = [branch("main", "moved")]
        client.commit_values = [{"hash": "moved", "date": DATE}]
        stream = self.build(stream_class, client, repo)
        stream.state = synced_state(repo, field, self.known(stream_class), "older")

        read_all_buckets(stream)

        stored = stream.state["repositories"][repo_state_key(repo)][field]
        assert stored == (["moved"] if field == "head_shas" else {"main": "moved"})


class TestBranchSnapshotsSurviveAnEmptyListing:
    """A branch snapshot replaces the previous one, so publishing an empty one
    deletes every branch the repository had."""

    def build(self, client, repo):
        return BranchesStream(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})

    def synced(self, client, repo):
        stream = self.build(client, repo)
        stream.state = {}
        read_all_buckets(stream)
        return stream.state

    def test_an_empty_listing_publishes_no_snapshot(self, repo):
        client = FakeClient()
        client.branch_values[repo.uuid] = [branch("main", "a1")]
        state = self.synced(client, repo)

        client.branch_values[repo.uuid] = []
        pushed = repository(updated_on="2026-07-01T00:00:00+00:00")
        second = self.build(client, pushed)
        second.state = state

        records = read_all_buckets(second)

        assert records == [], "neither items nor a marker: the snapshot would read as 'no branches'"
        assert second.state["repositories"][repo_state_key(pushed)]["repo_updated_on"] != "2026-07-01T00:00:00+00:00", (
            "and the cursor must stay open so the listing is retried"
        )

    def test_an_empty_repository_publishes_once_the_answer_repeats(self, repo):
        """A second consecutive empty listing is the repository, not the API."""
        client = FakeClient()
        client.branch_values[repo.uuid] = []
        first = self.build(client, repo)
        first.state = {}

        assert read_all_buckets(first) == [], "one empty answer is only an observation"

        second = self.build(client, repo)
        second.state = first.state
        records = read_all_buckets(second)

        markers = [r for r in records if r["record_type"] == "snapshot_complete"]
        assert markers and markers[0]["snapshot_item_count"] == 0
        assert markers[0]["snapshot_available"] is True

    def test_state_written_before_this_rule_is_not_trusted_into_a_deletion(self, repo):
        """Deployed state carries no branch count, so a first empty listing
        under the new code must not read as 'this repository has no branches'."""
        client = FakeClient()
        client.branch_values[repo.uuid] = []
        stream = self.build(client, repo)
        stream.state = {
            "version": STATE_VERSION,
            "bucket_count": BUCKET_COUNT,
            "repositories": {repo_state_key(repo): {"repo_updated_on": "older"}},
        }

        records = read_all_buckets(stream)

        assert records == []
        assert stream.state["repositories"][repo_state_key(repo)]["repo_updated_on"] == "older", (
            "the cursor must stay open so the listing is retried"
        )


@pytest.mark.parametrize("stream_class", list(HEAD_FIELD))
class TestAFirstEmptyListingIsNotTrusted:
    """On fresh state an empty listing is indistinguishable from a glitch, and
    trusting it advances the cursor with no heads — the idle gate then skips the
    repository until somebody pushes to it."""

    def build(self, stream_class, client, repo):
        stream = stream_class(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})
        stream.state = {}
        return stream

    def test_the_cursor_stays_open(self, stream_class, repo):
        client = FakeClient()
        client.branch_values[repo.uuid] = []

        stream = self.build(stream_class, client, repo)
        read_all_buckets(stream)

        stored = stream.state["repositories"][repo_state_key(repo)]
        assert stored["repo_updated_on"] == "", "an unconfirmed empty listing must not close the idle gate"

    def test_the_repository_is_read_again_without_a_push(self, stream_class, repo):
        client = FakeClient()
        client.branch_values[repo.uuid] = []
        first = self.build(stream_class, client, repo)
        read_all_buckets(first)

        client.branch_values[repo.uuid] = [branch("main", "fresh")]
        client.commit_values = [{"hash": "fresh", "date": DATE}]
        second = self.build(stream_class, client, repo)
        second.state = first.state

        records = read_all_buckets(second)

        assert records, "the reappearing branch must be picked up"
        assert client.commit_calls, "and its range actually fetched"

    def test_a_repeated_empty_listing_finally_settles(self, stream_class, repo):
        client = FakeClient()
        client.branch_values[repo.uuid] = []
        first = self.build(stream_class, client, repo)
        read_all_buckets(first)

        second = self.build(stream_class, client, repo)
        second.state = first.state
        read_all_buckets(second)

        stored = second.state["repositories"][repo_state_key(repo)]
        assert stored["repo_updated_on"] == repo.raw["updated_on"], (
            "the same answer twice is the repository, not the API"
        )
