"""Request-budget guarantees for large fleets (rate-limit survival).

Two mechanisms, pinned separately:

* idle gate — a repository whose `updated_on` (free with the workspace listing)
  has not changed since the last pass costs the push-driven streams ZERO
  requests: no branch listing, no commit range, no diffstat.
* shared selections — the PR / pipeline / issue listing happens once per
  repository per sync and is shared through the catalog as a SLIM projection,
  instead of each stream in the family re-listing (six times for PRs).

Both matter at ~1,400 repositories against Bitbucket's ~1,000 req/h budget.
"""

from __future__ import annotations

from airbyte_cdk.models import SyncMode

from source_bitbucket_cloud.streams.base import BUCKET_COUNT, repo_state_key
from source_bitbucket_cloud.streams.branches import BranchesStream
from source_bitbucket_cloud.streams.commits import CommitsStream
from source_bitbucket_cloud.streams.file_changes import FileChangesStream
from source_bitbucket_cloud.streams.pr_comments import PRCommentsStream
from source_bitbucket_cloud.streams.pr_commits import PRCommitsStream
from source_bitbucket_cloud.streams.pull_requests import PullRequestsStream
from tests.conftest import SHARED, FakeCatalog, FakeClient, branch

UPDATED = "2026-06-01T00:00:00+00:00"


class CountingClient(FakeClient):
    def __init__(self):
        super().__init__()
        self.branch_calls = 0
        self.pr_list_calls = 0

    def branches(self, repo):
        self.branch_calls += 1
        return self.branch_values.get(repo.uuid, [])

    def paginate(self, path, **kwargs):
        if path.endswith("pullrequests"):
            self.pr_list_calls += 1
        return super().paginate(path, **kwargs)


def read_all(stream, repo):
    records = []
    for bucket in range(BUCKET_COUNT):
        records.extend(stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": bucket}))
    return records


class TestIdleGate:
    def _synced_state(self, repo, extra=None):
        return {
            "version": 3,
            "bucket_count": 8,
            "repositories": {repo_state_key(repo): {"repo_updated_on": UPDATED, **(extra or {})}},
        }

    def test_unchanged_repository_costs_zero_requests(self, repo):
        client = CountingClient()
        client.branch_values[repo.uuid] = [branch("main", "a1")]
        for cls, extra in (
            (CommitsStream, {"head_shas": ["a1"]}),
            (FileChangesStream, {"head_shas": ["a1"]}),
            (BranchesStream, {}),
        ):
            stream = cls(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})
            stream.state = self._synced_state(repo, extra)

            records = read_all(stream, repo)

            assert records == [], f"{cls.__name__} emitted for an idle repository"
        assert client.branch_calls == 0, "an idle repository must not be listed at all"
        assert client.commit_calls == []

    def test_changed_updated_on_syncs_again(self, repo):
        client = CountingClient()
        client.branch_values[repo.uuid] = [branch("main", "a2")]
        client.commit_values = [{"hash": "a2", "date": "2026-07-01T00:00:00+00:00"}]
        stream = CommitsStream(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})
        stream.state = {
            "version": 3,
            "bucket_count": 8,
            "repositories": {
                repo_state_key(repo): {"head_shas": ["a1"], "repo_updated_on": "2026-05-01T00:00:00+00:00"}
            },
        }

        records = read_all(stream, repo)

        assert [r["hash"] for r in records] == ["a2"]
        assert stream.state["repositories"][repo_state_key(repo)]["repo_updated_on"] == UPDATED

    def test_first_sync_is_never_gated(self, repo):
        client = CountingClient()
        client.branch_values[repo.uuid] = [branch("main", "a1")]
        client.commit_values = [{"hash": "a1", "date": UPDATED}]
        stream = CommitsStream(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})
        stream.state = {}

        records = read_all(stream, repo)

        assert len(records) == 1

    def test_legacy_state_without_the_field_is_not_gated(self, repo):
        """Migrated pre-rewrite state has head_shas but no repo_updated_on: the
        first pass must run (and thereby stamp the field)."""
        client = CountingClient()
        client.branch_values[repo.uuid] = [branch("main", "new")]
        client.commit_values = [{"hash": "new", "date": UPDATED}]
        stream = CommitsStream(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})
        stream.state = {"ws/repo/main": {"head_sha": "old", "date": "2026-05-01T00:00:00+00:00"}}

        records = read_all(stream, repo)

        assert [r["hash"] for r in records] == ["new"]
        assert client.commit_calls == [(["new"], ["old"])], "still resumes from the migrated heads"


def pr(pr_id=42):
    return {
        "id": pr_id,
        "title": "t",
        "description": "x" * 5000,
        "state": "MERGED",
        "updated_on": "2026-06-30T00:00:00+00:00",
        "created_on": UPDATED,
        "source": {"branch": {"name": "f"}, "commit": {"hash": "src"}},
        "destination": {"branch": {"name": "main"}, "commit": {"hash": "dst"}},
        "participants": [{"user": {"uuid": "{u}"}, "role": "REVIEWER"}] * 20,
    }


class TestSharedPrSelection:
    def _run(self, cls, repo, client, catalog):
        stream = cls(**{**SHARED, "client": client, "catalog": catalog})
        stream.state = {}
        return read_all(stream, repo)

    def test_children_reuse_the_parents_listing(self, repo):
        client = CountingClient()
        client.pr_values = [pr()]
        catalog = FakeCatalog([repo], client)

        self._run(PullRequestsStream, repo, client, catalog)
        after_parent = client.pr_list_calls
        self._run(PRCommentsStream, repo, client, catalog)
        self._run(PRCommitsStream, repo, client, catalog)

        assert after_parent >= 1
        assert client.pr_list_calls == after_parent, (
            "child streams re-listed pull requests instead of reusing the shared selection"
        )

    def test_cache_holds_slim_projections_only(self, repo):
        """The memory guard: raw PR objects (description, participants, …) held
        for every repository across six sequential streams would cost hundreds
        of MB. Only four whitelisted fields may be cached."""
        client = CountingClient()
        client.pr_values = [pr()]
        catalog = FakeCatalog([repo], client)

        self._run(PullRequestsStream, repo, client, catalog)

        assert catalog.pr_selections, "the parent must fill the cache"
        for slim_list, _state in catalog.pr_selections.values():
            for entry in slim_list:
                assert set(entry) == {"id", "updated_on", "source", "destination"}, entry
                assert set(entry["source"]) == {"commit"}

    def test_children_produce_identical_records_from_the_slim_cache(self, repo):
        """Equivalence: a child fed from the cache emits exactly what it would
        have emitted from its own fetch."""
        client_cached = CountingClient()
        client_cached.pr_values = [pr()]
        comments = (True, [{"id": 7, "content": {"raw": "lgtm"}, "user": {"uuid": "{u}"}}])
        client_cached.optional_values["repositories/ws/repo/pullrequests/42/comments"] = comments
        catalog = FakeCatalog([repo], client_cached)
        self._run(PullRequestsStream, repo, client_cached, catalog)  # fills cache
        from_cache = self._run(PRCommentsStream, repo, client_cached, catalog)

        client_fresh = CountingClient()
        client_fresh.pr_values = [pr()]
        client_fresh.optional_values["repositories/ws/repo/pullrequests/42/comments"] = (
            True, [{"id": 7, "content": {"raw": "lgtm"}, "user": {"uuid": "{u}"}}],
        )
        fresh = self._run(PRCommentsStream, repo, client_fresh, FakeCatalog([repo], client_fresh))

        # generation_id is derived from the stream instance's run id, so it (and
        # unique_key, which embeds it) legitimately differs between two runs;
        # the equivalence claim is about entity content.
        volatile = {"collected_at", "generation_id", "unique_key"}
        strip = lambda rows: [{k: v for k, v in r.items() if k not in volatile} for r in rows]
        assert strip(from_cache) == strip(fresh)

    def test_divergent_watermark_fetches_its_own(self, repo):
        """A stream whose state lags (failed last sync) must not reuse a
        narrower selection."""
        client = CountingClient()
        client.pr_values = [pr()]
        catalog = FakeCatalog([repo], client)
        self._run(PullRequestsStream, repo, client, catalog)
        after_parent = client.pr_list_calls

        lagging = PRCommentsStream(**{**SHARED, "client": client, "catalog": catalog})
        lagging.state = {
            "version": 3,
            "bucket_count": 8,
            "repositories": {repo_state_key(repo): {"updated_on": "2026-01-01T00:00:00+00:00", "reconcile_after_id": 0}},
        }
        read_all(lagging, repo)

        assert client.pr_list_calls > after_parent, "a divergent watermark must trigger its own fetch"
