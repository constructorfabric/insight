"""A repository the token cannot read must not fail the sync.

A repository can be listed for a workspace and still answer 403 to every request
under it — routine with repo-scoped tokens and per-repository permissions.
Retrying never changes it, so treating it as a failure leaves the sync red on
every run and buries the transient failures that do need attention. These tests
pin the distinction: denied is skipped, everything else still fails loudly.

The matrix classes at the bottom run EVERY stream the source wires — the list is
derived from `SourceBitbucketCloud.streams()` itself, so a stream added later is
covered automatically instead of depending on this file being remembered.
"""

from __future__ import annotations

import pytest
from airbyte_cdk.models import SyncMode

from source_bitbucket_cloud.client import BitbucketApiError
from source_bitbucket_cloud.source import SourceBitbucketCloud
from source_bitbucket_cloud.streams.base import BUCKET_COUNT, repo_state_key, repository_bucket
from source_bitbucket_cloud.streams.branches import BranchesStream
from source_bitbucket_cloud.streams.commits import CommitsStream
from tests.conftest import SHARED, FakeCatalog, FakeClient, branch, repository


def every_stream_class():
    """All stream classes, from the source's own wiring — not a hand list."""
    source = SourceBitbucketCloud()
    streams = source.streams(
        {
            "bitbucket_token": "t",
            "bitbucket_workspaces": ["ws"],
            "insight_tenant_id": "T",
            "insight_source_id": "S",
        }
    )
    return [type(stream) for stream in streams]


def denied(status: int):
    class DeniedClient(FakeClient):
        def branches(self, repo):
            raise BitbucketApiError(status, "https://api.bitbucket.org/2.0/x", "no access")

    return DeniedClient()


def read_all_buckets(stream):
    records, error = [], None
    for bucket in range(BUCKET_COUNT):
        try:
            records.extend(stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": bucket}))
        except RuntimeError as exc:
            error = exc
    return records, error


def build(cls, repos, client):
    catalog = FakeCatalog(repos, client)
    return cls(**{**SHARED, "client": client, "catalog": catalog}), catalog


class TestDeniedRepositoryIsSkipped:
    def test_403_does_not_fail_the_sync(self):
        stream, _ = build(CommitsStream, [repository()], denied(403))
        stream.state = {}

        records, error = read_all_buckets(stream)

        assert error is None, "a permanently denied repository must not fail the sync"
        assert records == []

    def test_404_does_not_fail_the_sync(self):
        """A repository deleted between the listing and the fetch."""
        stream, _ = build(CommitsStream, [repository()], denied(404))
        stream.state = {}

        _, error = read_all_buckets(stream)

        assert error is None

    def test_denied_repository_state_is_not_advanced(self):
        stream, _ = build(CommitsStream, [repository()], denied(403))
        stream.state = {}

        read_all_buckets(stream)

        assert stream.state["repositories"] == {}

    def test_it_is_recorded_on_the_shared_catalog(self):
        """So the remaining streams skip it instead of rediscovering the 403."""
        repo = repository()
        stream, catalog = build(CommitsStream, [repo], denied(403))
        stream.state = {}

        read_all_buckets(stream)

        assert catalog.is_inaccessible(repo)
        assert catalog.inaccessible_count == 1

    def test_a_stream_started_later_skips_it_without_a_request(self):
        repo = repository()
        client = denied(403)
        catalog = FakeCatalog([repo], client)
        catalog.mark_inaccessible(repo)
        later = CommitsStream(**{**SHARED, "client": client, "catalog": catalog})
        later.state = {}

        records, error = read_all_buckets(later)

        assert (records, error) == ([], None)
        assert later._skipped_repositories == [f"{repo.workspace}/{repo.slug}"], (
            "a pre-known inaccessible repository must still appear in this stream's skipped summary"
        )

    def test_other_repositories_still_sync(self):
        good, bad = repository(slug="good"), repository(slug="bad", uuid="{bad}")

        class MixedClient(FakeClient):
            def branches(self, repo):
                if repo.slug == "bad":
                    raise BitbucketApiError(403, "https://api.bitbucket.org/2.0/x", "no access")
                return [branch("main", "a1")]

        client = MixedClient()
        client.commit_values = [{"hash": "a1", "date": "2026-06-01T00:00:00+00:00"}]
        stream, _ = build(CommitsStream, [good, bad], client)
        stream.state = {}

        records, error = read_all_buckets(stream)

        assert error is None
        assert [r["hash"] for r in records] == ["a1"]
        assert stream.state["repositories"] == {repo_state_key(good): {"head_shas": ["a1"], "repo_updated_on": "2026-06-01T00:00:00+00:00"}}


class TestTransientFailuresStillFail:
    def test_500_still_fails_the_sync(self):
        stream, catalog = build(CommitsStream, [repository()], denied(500))
        stream.state = {}

        _, error = read_all_buckets(stream)

        assert error is not None, "a transient failure must still surface"
        assert not catalog.is_inaccessible(repository()), "and must not mark the repository denied"

    def test_non_api_errors_still_fail_the_sync(self):
        class BrokenClient(FakeClient):
            def branches(self, repo):
                raise RuntimeError("boom")

        stream, _ = build(CommitsStream, [repository()], BrokenClient())
        stream.state = {}

        _, error = read_all_buckets(stream)

        assert error is not None


class TestBranchesSnapshotStaysSafe:
    """branches is a per-repository, deletion-aware snapshot.

    A denied repository must produce NO marker — its previous generation then
    stays the newest complete one and its branches are retained. Emitting an
    available marker instead would read as "every branch of that repository was
    deleted". Per-repository (not bucket) scope matters at fleet scale: with
    denied repositories scattered across buckets, a bucket-scoped generation
    would freeze branch updates for every repository, permanently.
    """

    def _bucket_of(self, repo):
        return repository_bucket(repo_state_key(repo))

    def test_denied_repository_produces_no_marker_and_keeps_its_generation(self):
        repo = repository()
        stream, _ = build(BranchesStream, [repo], denied(403))

        records = list(
            stream.read_records(SyncMode.full_refresh, stream_slice={"bucket_id": self._bucket_of(repo)})
        )

        assert records == [], "a denied repository must contribute nothing — absence keeps its previous generation"

    def test_denied_repository_does_not_freeze_its_neighbours(self):
        """The fleet-scale property: other repositories keep updating."""
        readable = repository(slug="readable")
        stream_denied = repository(slug="denied", uuid="{denied}")
        # force both into the same bucket so the old bucket-scope design would couple them
        while repository_bucket(repo_state_key(readable)) != repository_bucket(repo_state_key(stream_denied)):
            readable = repository(slug=readable.slug + "x")

        class MixedClient(FakeClient):
            def branches(self, repo):
                if repo.slug.startswith("denied"):
                    raise BitbucketApiError(403, "https://api.bitbucket.org/2.0/x", "no access")
                return [branch("main", "a1")]

        stream, _ = build(BranchesStream, [readable, stream_denied], MixedClient())
        records = list(
            stream.read_records(SyncMode.full_refresh, stream_slice={"bucket_id": self._bucket_of(readable)})
        )

        markers = [r for r in records if r.get("record_type") == "snapshot_complete"]
        assert len(markers) == 1, "exactly the readable repository closes a generation"
        assert markers[0]["repo_slug"] == readable.slug
        assert markers[0]["snapshot_available"] is True
        assert markers[0]["snapshot_item_count"] == 1

    def test_marker_is_available_when_every_repository_was_read(self):
        repo = repository()
        client = FakeClient()
        client.branch_values[repo.uuid] = [branch("main", "a1")]
        stream, _ = build(BranchesStream, [repo], client)

        records = list(
            stream.read_records(SyncMode.full_refresh, stream_slice={"bucket_id": self._bucket_of(repo)})
        )

        assert records[-1]["snapshot_available"] is True
        assert records[-1]["snapshot_item_count"] == 1

    def test_a_denied_repository_does_not_fail_the_branches_stream(self):
        stream, _ = build(BranchesStream, [repository()], denied(403))

        _, error = read_all_buckets(stream)

        assert error is None


class FullyDeniedClient(FakeClient):
    """Faithful mirror of the real client against an all-403 repository.

    Raising paths raise BitbucketApiError(403); tolerant paths behave as the
    real client does — paginate_optional answers (False, ()) and a request made
    with allow_statuses covering 403 answers None.
    """

    def _deny(self):
        raise BitbucketApiError(403, "https://api.bitbucket.org/2.0/x", "no access")

    def branches(self, repo):
        self._deny()

    def commits_between(self, repo, include, exclude):
        self._deny()

    def paginate(self, path, **kwargs):
        self._deny()

    def paginate_optional(self, path, **kwargs):
        return False, iter(())

    def request(self, method, path, **kwargs):
        allowed = kwargs.get("allow_statuses") or ()
        if 403 in allowed:
            return None
        self._deny()


class BrokenClient(FakeClient):
    """Every request fails with a retry-exhausted 500 — a transient outage."""

    def _boom(self):
        raise BitbucketApiError(500, "https://api.bitbucket.org/2.0/x", "server error")

    branches = lambda self, repo: self._boom()  # noqa: E731
    commits_between = lambda self, repo, include, exclude: self._boom()  # noqa: E731
    paginate = lambda self, path, **kw: self._boom()  # noqa: E731
    paginate_optional = lambda self, path, **kw: self._boom()  # noqa: E731
    request = lambda self, method, path, **kw: self._boom()  # noqa: E731


@pytest.mark.parametrize("stream_class", every_stream_class(), ids=lambda c: c.__name__)
class TestEveryStreamSurvivesADeniedRepository:
    """The no-gaps guarantee: no stream may fail the sync over a 403 repository."""

    def test_denied_repository_never_fails_the_sync(self, stream_class):
        repo = repository()
        client = FullyDeniedClient()
        stream = stream_class(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})
        if hasattr(stream, "state"):
            stream.state = {}

        records, error = read_all_buckets(stream)

        assert error is None, f"{stream_class.__name__} failed the sync over a denied repository"
        items = [r for r in records if r.get("record_type") == "item"]
        if stream_class.__name__ == "RepositoriesStream":
            # Its data comes from the workspace listing, which succeeded — the
            # repository is visible, only its contents are not. Emitting the
            # metadata is correct.
            assert items, "the workspace listing was readable; the repository row should be emitted"
        else:
            assert items == [], f"{stream_class.__name__} emitted items from a repository it could not read"
        # Any marker touching the denied repository must say unavailable —
        # otherwise dbt treats the denied read as a legitimate empty collection
        # and deletes rows. Markers of unrelated empty buckets may stay
        # available: their partitions contain nothing to delete.
        repo_bucket = repository_bucket(repo_state_key(repo))
        touching = [
            m
            for m in records
            if m.get("record_type") == "snapshot_complete"
            and (m.get("repository_uuid") == repo.uuid or m.get("bucket_id") == repo_bucket)
        ]
        if stream_class.__name__ != "RepositoriesStream":
            assert all(m["snapshot_available"] is False for m in touching), (
                f"{stream_class.__name__} marked a denied read as an available snapshot"
            )

    def test_denied_repository_state_never_advances(self, stream_class):
        repo = repository()
        client = FullyDeniedClient()
        stream = stream_class(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})
        if not hasattr(stream, "state"):
            pytest.skip("full-refresh stream keeps no state")
        stream.state = {}

        read_all_buckets(stream)

        assert stream.state["repositories"].get(repo_state_key(repo), {}) in ({}, None) or (
            "head_shas" not in stream.state["repositories"].get(repo_state_key(repo), {})
            and "updated_on" not in stream.state["repositories"].get(repo_state_key(repo), {})
        ), f"{stream_class.__name__} advanced state for a repository it never read"


class TestCredentialFailureAbortsLoudly:
    """401 is global, not per-repository: quarantining every repo one by one
    would drown the log and end in a generic message. Abort at the first one
    with the actionable cause."""

    def test_401_aborts_immediately_with_the_cause(self):
        good, other = repository(slug="one"), repository(slug="two", uuid="{two}")
        stream, catalog = build(CommitsStream, [good, other], denied(401))
        stream.state = {}

        with pytest.raises(RuntimeError, match="authentication failed"):
            for bucket in range(BUCKET_COUNT):
                list(stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": bucket}))

        assert catalog.inaccessible_count == 0, "401 must not mark repositories denied — the token is the problem"
        assert stream._failed_repositories == [], "and must not be recorded as per-repository failures"


class TestVanishedDiffstatIsTolerated:
    """A commit's diffstat can be permanently gone (orphaned merge parents,
    rewritten history) — the pre-rewrite connector tolerated exactly this
    (`ignore_404`). It must mark that commit's snapshot unavailable, not fail
    the repository on every sync forever."""

    def test_missing_diffstat_marks_snapshot_unavailable_not_the_sync(self, repo):
        client = FakeClient()
        client.branch_values[repo.uuid] = [branch("main", "head")]
        client.commit_values = [{"hash": "gone", "date": "2026-06-01T00:00:00+00:00"}]
        # the diffstat endpoint answers 404 -> paginate_optional -> (False, ())
        client.optional_values[client.repo_path(repo, "diffstat/gone")] = (False, [])
        from source_bitbucket_cloud.streams.file_changes import FileChangesStream

        stream, catalog = build(FileChangesStream, [repo], client)
        stream.state = {}

        records, error = read_all_buckets(stream)

        assert error is None, "one vanished diffstat must not fail the repository forever"
        assert not catalog.is_inaccessible(repo)
        markers = [r for r in records if r.get("record_type") == "snapshot_complete"]
        assert markers and markers[0]["snapshot_available"] is False, (
            "the denial must be recorded — an available empty snapshot would read as "
            "'this commit changed nothing' and zero its line counts"
        )
        assert stream.state["repositories"][repo_state_key(repo)]["head_shas"] == ["head"], (
            "the repository still advances past the bad commit"
        )


class TestManyBranchRepositoriesChunkTheCommitRange:
    """Bitbucket's include/exclude ceiling is undocumented (BCLOUD-13229); a
    repository with hundreds of branches must not send them in one form."""

    def test_includes_are_chunked_and_excludes_ride_along(self):
        from source_bitbucket_cloud.client import BitbucketClient

        client = BitbucketClient("tok")
        calls: list[list[tuple[str, str]]] = []

        def fake_paginate(path, *, params=None, method="GET", data=None, **kwargs):
            calls.append(list(data or []))
            return iter(())

        client.paginate = fake_paginate
        includes = [f"new{i:04d}" for i in range(250)]
        excludes = [f"old{i:04d}" for i in range(30)]

        list(client.commits_between(repository(), includes, excludes))

        assert len(calls) == 3  # 250 heads / 100 per chunk
        for form in calls:
            chunk_includes = [v for k, v in form if k == "include"]
            chunk_excludes = sorted(v for k, v in form if k == "exclude")
            assert len(chunk_includes) <= 100
            assert chunk_excludes == sorted(excludes), "every chunk must carry the FULL exclude set"
        fetched = sorted(v for form in calls for k, v in form if k == "include")
        assert fetched == sorted(includes), "the union of chunks must cover every head exactly once"


class TestFeatureLevelDenialStaysFeatureLevel:
    """Pipelines is a per-repository feature: a 403 there means "no pipelines
    visible", not "this repository is unreadable". It must not mark the whole
    repository inaccessible — that would suppress its commits and pull requests.

    These paths only run with pre-existing pipeline state, which the all-denied
    matrix above never reaches, so they are pinned separately.
    """

    def test_open_pipeline_refetch_403_does_not_poison_the_repository(self):
        from source_bitbucket_cloud.streams.metric_events import PipelinesStream

        repo = repository()

        class PipelinesDeniedClient(FakeClient):
            def request(self, method, path, **kwargs):
                allowed = kwargs.get("allow_statuses") or ()
                if 403 in allowed:
                    return None  # what the real client answers for a tolerated 403
                raise BitbucketApiError(403, path, "no access")

        client = PipelinesDeniedClient()
        client.optional_values["repositories/ws/repo/pipelines"] = (True, [])
        catalog = FakeCatalog([repo], client)
        stream = PipelinesStream(**{**SHARED, "client": client, "catalog": catalog})
        stream.state = {
            "version": 3,
            "bucket_count": 8,
            "repositories": {repo_state_key(repo): {"created_on": "2026-06-01T00:00:00+00:00", "open": ["p1"]}},
        }

        _, error = read_all_buckets(stream)

        assert error is None
        assert not catalog.is_inaccessible(repo), (
            "a pipelines-only denial must not mark the repository inaccessible"
        )

    def test_test_reports_403_marks_the_snapshot_not_the_repository(self):
        from source_bitbucket_cloud.streams.metric_events import PipelineStepTestReportsStream

        repo = repository()

        class ReportsDeniedClient(FakeClient):
            def request(self, method, path, **kwargs):
                allowed = kwargs.get("allow_statuses") or ()
                if 403 in allowed:
                    return None
                raise BitbucketApiError(403, path, "no access")

        client = ReportsDeniedClient()
        client.optional_values["repositories/ws/repo/pipelines"] = (
            True,
            [{"uuid": "p1", "created_on": "2026-06-02T00:00:00+00:00", "state": {"name": "COMPLETED"}}],
        )
        client.optional_values["repositories/ws/repo/pipelines/p1/steps"] = (True, [{"uuid": "s1"}])
        catalog = FakeCatalog([repo], client)
        stream = PipelineStepTestReportsStream(**{**SHARED, "client": client, "catalog": catalog})
        stream.state = {}

        records, error = read_all_buckets(stream)

        assert error is None
        assert not catalog.is_inaccessible(repo)
        markers = [r for r in records if r.get("record_type") == "snapshot_complete"]
        assert markers and all(m["snapshot_available"] is False for m in markers)


@pytest.mark.parametrize("stream_class", every_stream_class(), ids=lambda c: c.__name__)
class TestEveryStreamSurfacesTransientFailures:
    """The counterpart: a real outage must never be silently absorbed."""

    def test_500_fails_the_sync_loudly(self, stream_class):
        repo = repository()
        client = BrokenClient()
        stream = stream_class(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})
        if hasattr(stream, "state"):
            stream.state = {}

        _, error = read_all_buckets(stream)

        if type(stream).__name__ == "RepositoriesStream":
            pytest.skip("reads only the already-fetched catalog; no per-repository request to fail")
        assert error is not None, f"{stream_class.__name__} silently swallowed a 500"
