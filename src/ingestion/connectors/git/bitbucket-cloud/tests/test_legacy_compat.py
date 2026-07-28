"""Compatibility with data and state written by the pre-rewrite connector.

An upgraded instance must keep the rows it already has and carry on from its last
checkpoint, so two things have to hold:

  * a re-synced entity must land on the SAME `unique_key` the old connector
    wrote, otherwise the old row is never superseded and every metric counting
    rows (lines, pull requests) doubles;
  * the old flat state must still be understood, otherwise the head-set diff
    starts from nothing and re-reads all of history.

The expected keys here are written out literally, in the old connector's format
(`tenant:source:workspace:slug:...`), rather than derived from the helper — a
test that builds the key the same way the code does would pass no matter what
the format became.
"""

from __future__ import annotations

from airbyte_cdk.models import SyncMode

from source_bitbucket_cloud.streams.base import repo_state_key, repository_bucket
from source_bitbucket_cloud.streams.commits import CommitsStream
from source_bitbucket_cloud.streams.pr_comments import PRCommentsStream
from source_bitbucket_cloud.streams.pull_requests import PullRequestsStream
from tests.conftest import SHARED, FakeCatalog, FakeClient, branch, repository


def stream_for(cls, repo, client):
    return cls(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})


def read(stream, repo):
    return list(
        stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": repository_bucket(repo_state_key(repo))})
    )


def pr(pr_id=42):
    return {
        "id": pr_id,
        "title": "t",
        "state": "MERGED",
        "created_on": "2026-06-01T00:00:00+00:00",
        "updated_on": "2026-06-30T00:00:00+00:00",
        "source": {"branch": {"name": "f"}, "commit": {"hash": "s"}},
        "destination": {"branch": {"name": "main"}, "commit": {"hash": "d"}},
    }


class TestEntityKeysMatchPreRewrite:
    """Keys the pre-rewrite connector wrote, spelled out literally."""

    def test_commit(self, repo):
        client = FakeClient()
        client.branch_values[repo.uuid] = [branch("main", "a1")]
        client.commit_values = [{"hash": "a1", "date": "2026-06-01T00:00:00+00:00"}]
        stream = stream_for(CommitsStream, repo, client)
        stream.state = {}

        record = read(stream, repo)[0]

        assert record["entity_key"] == "T:S:ws:repo:a1"
        assert record["unique_key"] == "T:S:ws:repo:a1"

    def test_pull_request(self, repo):
        client = FakeClient()
        client.pr_values = [pr(42)]
        stream = stream_for(PullRequestsStream, repo, client)
        stream.state = {}

        record = read(stream, repo)[0]

        assert record["entity_key"] == "T:S:ws:repo:42"

    def test_pull_request_comment(self, repo):
        client = FakeClient()
        client.pr_values = [pr(42)]
        client.optional_values[client.repo_path(repo, "pullrequests/42/comments")] = (
            True,
            [{"id": 7, "content": {"raw": "lgtm"}, "user": {"uuid": "{u}"}}],
        )
        stream = stream_for(PRCommentsStream, repo, client)
        stream.state = {}

        item = next(r for r in read(stream, repo) if r["record_type"] == "item")

        assert item["entity_key"] == "T:S:ws:repo:42:7"

    def test_repository_uuid_is_still_carried_as_data(self, repo):
        """Re-keying must not lose the uuid — the dbt models join on this column."""
        client = FakeClient()
        client.branch_values[repo.uuid] = [branch("main", "a1")]
        client.commit_values = [{"hash": "a1", "date": "2026-06-01T00:00:00+00:00"}]
        stream = stream_for(CommitsStream, repo, client)
        stream.state = {}

        record = read(stream, repo)[0]

        assert record["repository_uuid"] == repo.uuid


class TestLegacyStateResumes:
    LEGACY_COMMITS_STATE = {
        # exactly what the pre-rewrite connector persisted: one entry per branch
        "ws/repo/main": {"date": "2026-06-01T00:00:00+00:00", "head_sha": "old-main"},
        "ws/repo/release": {"date": "2026-05-01T00:00:00+00:00", "head_sha": "old-release"},
    }

    def test_branch_heads_become_the_exclude_set(self, repo):
        """The whole point: only new commits are fetched, not all of history."""
        client = FakeClient()
        client.branch_values[repo.uuid] = [branch("main", "new-main"), branch("release", "old-release")]
        client.commit_values = [{"hash": "new1", "date": "2026-07-01T00:00:00+00:00"}]
        stream = stream_for(CommitsStream, repo, client)

        stream.state = dict(self.LEGACY_COMMITS_STATE)
        read(stream, repo)

        assert client.commit_calls, "no commit range was requested"
        include, exclude = client.commit_calls[0]
        assert sorted(exclude) == ["old-main", "old-release"]
        assert sorted(include) == ["new-main", "old-release"]

    def test_legacy_state_is_reshaped_not_discarded(self, repo):
        stream = stream_for(CommitsStream, repo, FakeClient())

        stream.state = dict(self.LEGACY_COMMITS_STATE)

        assert stream.state["repositories"] == {
            "ws/repo": {"head_shas": ["old-main", "old-release"]}
        }

    def test_pull_request_watermark_survives(self, repo):
        """Old per-PR entries collapse to the repository's newest cursor."""
        stream = stream_for(PullRequestsStream, repo, FakeClient())

        stream.state = {
            "ws/repo/41": {"pull_request_updated_on": "2026-06-01T00:00:00+00:00"},
            "ws/repo/42": {"pull_request_updated_on": "2026-06-30T00:00:00+00:00"},
        }

        assert stream.state["repositories"]["ws/repo"]["updated_on"] == "2026-06-30T00:00:00+00:00"
        assert stream.state["repositories"]["ws/repo"]["reconcile_after_id"] == 0

    def test_repository_level_cursor_survives(self, repo):
        stream = stream_for(PullRequestsStream, repo, FakeClient())

        stream.state = {"ws/repo": {"updated_on": "2026-06-30T00:00:00+00:00"}}

        assert stream.state["repositories"]["ws/repo"]["updated_on"] == "2026-06-30T00:00:00+00:00"

    def test_unparseable_partition_is_skipped_not_fatal(self, repo):
        """A repository that looks unsynced is re-fetched, which is safe."""
        stream = stream_for(CommitsStream, repo, FakeClient())

        stream.state = {"garbage": {"head_sha": "x"}, "ws/repo/main": {"head_sha": "keep"}}

        assert stream.state["repositories"] == {"ws/repo": {"head_shas": ["keep"]}}

    def test_uuid_keyed_state_from_the_rewrite_is_reset(self, repo):
        """Version 2 addressed repositories by uuid, so it no longer resolves."""
        stream = stream_for(CommitsStream, repo, FakeClient())

        stream.state = {"version": 2, "bucket_count": 8, "repositories": {repo.uuid: {"head_shas": ["a"]}}}

        assert stream.state["repositories"] == {}
