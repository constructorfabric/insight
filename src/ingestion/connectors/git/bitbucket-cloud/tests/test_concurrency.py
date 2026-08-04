from __future__ import annotations

import threading
import time
from contextlib import closing

import pytest
from source_bitbucket_cloud.client import BitbucketApiError, RepositoryRef
from source_bitbucket_cloud.streams.base import (
    BUCKET_COUNT,
    QUEUE_POLL_SECONDS,
    RECORD_BUFFER,
    repo_state_key,
    repository_bucket,
)
from source_bitbucket_cloud.streams.commits import CommitsStream
from tests.conftest import SHARED, FakeCatalog, FakeClient, branch, repository

DATE = "2026-06-01T00:00:00+00:00"
VOLATILE = {"collected_at", "generation_id", "unique_key"}


def fleet(size: int = 12):
    return [repository(slug=f"repo{index:02d}", uuid=f"{{r-{index}}}") for index in range(size)]


def fleet_in_one_bucket(size: int, bucket: int = 0) -> list[RepositoryRef]:
    """Repositories that all land in the same slice, so one read_records call
    really does run several workers."""
    repos: list[RepositoryRef] = []
    for index in range(size * BUCKET_COUNT * 20):
        repo = repository(slug=f"same{index:03d}", uuid=f"{{s-{index}}}")
        if repository_bucket(repo_state_key(repo)) == bucket:
            repos.append(repo)
        if len(repos) == size:
            return repos
    raise AssertionError(f"could not place {size} repositories in bucket {bucket}")


class FleetClient(FakeClient):
    """One branch and one commit per repository, so a record identifies its
    repository unambiguously."""

    def __init__(self, repos, delay: float = 0.0):
        super().__init__()
        self.delay = delay
        self.threads: set[int] = set()
        self._lock = threading.Lock()
        for repo in repos:
            self.branch_values[repo.uuid] = [branch("main", f"head-{repo.slug}")]

    def branches(self, repo):
        with self._lock:
            self.threads.add(threading.get_ident())
        if self.delay:
            time.sleep(self.delay)
        return self.branch_values.get(repo.uuid, [])

    def commits_between(self, repo, include, exclude):
        with self._lock:
            self.commit_calls.append((list(include), list(exclude)))
        return iter([{"hash": sha, "date": DATE} for sha in include])


def read_all_buckets(stream):
    records = []
    for bucket in range(BUCKET_COUNT):
        records.extend(stream.read_records(None, stream_slice={"bucket_id": bucket}))
    return records


def build(repos, client, concurrency: int):
    stream = CommitsStream(
        **{**SHARED, "concurrency": concurrency, "client": client, "catalog": FakeCatalog(repos, client)}
    )
    stream.state = {}
    return stream


def comparable(records):
    return sorted(
        tuple(sorted((k, str(v)) for k, v in record.items() if k not in VOLATILE)) for record in records
    )


class TestConcurrentReadsMatchSerialOnes:
    @pytest.mark.parametrize("concurrency", [2, 4, 8])
    def test_same_records_and_same_state(self, concurrency):
        repos = fleet()
        serial_client = FleetClient(repos)
        serial = build(repos, serial_client, 1)
        expected = read_all_buckets(serial)

        parallel_client = FleetClient(repos)
        parallel = build(repos, parallel_client, concurrency)
        actual = read_all_buckets(parallel)

        assert comparable(actual) == comparable(expected)
        assert parallel.state == serial.state
        assert len(parallel_client.commit_calls) == len(serial_client.commit_calls)

    def test_a_slow_repository_does_not_hold_back_finished_ones(self):
        """One deep history at the front of a bucket must not park the whole
        pool: the other repositories' records leave as soon as they are ready."""
        repos = fleet_in_one_bucket(4)
        slow = repos[0]
        gate = threading.Event()

        class SlowFirstClient(FleetClient):
            def branches(self, repo):
                if repo.uuid == slow.uuid:
                    gate.wait(timeout=30)
                return super().branches(repo)

        client = SlowFirstClient(repos)
        stream = build(repos, client, 4)
        records = stream.read_records(None, stream_slice={"bucket_id": 0})

        fast_slugs = {repo.slug for repo in repos[1:]}
        seen: set[str] = set()
        for record in records:
            seen.add(record["repo_slug"])
            if seen >= fast_slugs:
                break
        assert seen >= fast_slugs and slow.slug not in seen, (
            "finished repositories must drain while the slow one is still fetching"
        )

        gate.set()
        seen.update(record["repo_slug"] for record in records)
        assert slow.slug in seen, "and the slow repository still completes"

    def test_work_actually_runs_in_parallel(self):
        repos = fleet()
        client = FleetClient(repos, delay=0.002)

        read_all_buckets(build(repos, client, 8))

        assert len(client.threads) > 1, "the pool must do the fetching, not the consumer"

    def test_one_worker_stays_on_the_serial_path(self):
        repos = fleet()
        client = FleetClient(repos)

        read_all_buckets(build(repos, client, 1))

        assert client.threads == {threading.get_ident()}


class TestFailuresKeepTheirSemantics:
    def denied_client(self, repos, victim: str, status: int):
        class DeniedClient(FleetClient):
            def branches(self, repo):
                if repo.slug == victim:
                    raise BitbucketApiError(status, "https://api.bitbucket.org/2.0/x", "denied")
                return super().branches(repo)

        return DeniedClient(repos)

    @pytest.mark.parametrize("status", [403, 404])
    def test_a_denied_repository_is_skipped_not_failed(self, status):
        repos = fleet()
        client = self.denied_client(repos, "repo05", status)
        stream = build(repos, client, 4)

        records = read_all_buckets(stream)

        assert stream._failed_repositories == [], f"HTTP {status} must not count as a failure"
        assert "ws/repo05" in stream._skipped_repositories, f"HTTP {status} must be recorded as skipped"
        assert {r["repo_slug"] for r in records} == {repo.slug for repo in repos} - {"repo05"}, (
            f"every other repository must still be read past an HTTP {status}"
        )

    def test_a_transient_failure_still_fails_the_sync(self):
        repos = fleet()
        client = self.denied_client(repos, "repo05", 500)
        stream = build(repos, client, 4)

        with pytest.raises(RuntimeError, match="repositories failed"):
            read_all_buckets(stream)

        assert stream._failed_repositories == ["ws/repo05"]
        healthy = [repo for repo in repos if repo.slug != "repo05"]
        assert all(repo_state_key(repo) in stream.state["repositories"] for repo in healthy), (
            "one repository's failure must not cost its neighbours their checkpoints"
        )

    def test_a_credential_failure_aborts(self):
        repos = fleet()
        client = self.denied_client(repos, "repo05", 401)
        stream = build(repos, client, 4)

        with pytest.raises(RuntimeError, match="authentication failed"):
            read_all_buckets(stream)

    def test_a_failed_repository_does_not_advance_its_state(self):
        repos = fleet()
        client = self.denied_client(repos, "repo05", 500)
        stream = build(repos, client, 4)

        with pytest.raises(RuntimeError):
            read_all_buckets(stream)

        victim = next(repo for repo in repos if repo.slug == "repo05")
        assert repo_state_key(victim) not in stream.state["repositories"]


class TestBackpressure:
    def test_a_worker_stops_at_the_buffer_instead_of_reading_ahead(self):
        """Two workers in one slice, each with more history than the buffer
        holds: they must park rather than grow, and lose nothing by parking."""
        repos = fleet_in_one_bucket(2)
        overflow = RECORD_BUFFER * 3

        class WideClient(FleetClient):
            def __init__(self, fleet_repos: list[RepositoryRef]) -> None:
                super().__init__(fleet_repos)
                self.produced = 0

            def commits_between(self, repo, include, exclude):
                def history():
                    for index in range(overflow):
                        with self._lock:
                            self.produced += 1
                        yield {"hash": f"{repo.slug}-{index}", "date": DATE}

                return history()

        client = WideClient(repos)
        stream = build(repos, client, 2)
        records = stream.read_records(None, stream_slice={"bucket_id": 0})

        next(records)
        parked = client.produced
        assert parked <= 2 * (RECORD_BUFFER + 1), (
            f"{parked} records fetched before the first was consumed; two buffers hold "
            f"{2 * RECORD_BUFFER}"
        )

        assert len(list(records)) + 1 == overflow * len(repos), "parking must not drop records"


class TestAbandoningTheReadTerminates:
    def test_closing_the_generator_releases_parked_workers(self):
        """Airbyte can stop reading mid-bucket; workers blocked on a full
        buffer must be released before the pool is joined."""
        repos = fleet_in_one_bucket(6)
        overflow = RECORD_BUFFER * 2

        class WideClient(FleetClient):
            def commits_between(self, repo, include, exclude):
                return iter([{"hash": f"{repo.slug}-{n}", "date": DATE} for n in range(overflow)])

        stream = build(repos, WideClient(repos), 4)
        finished = threading.Event()

        def read_a_little():
            records = stream.read_records(None, stream_slice={"bucket_id": 0})
            for _ in range(3):
                next(records, None)
            records.close()
            finished.set()

        reader = threading.Thread(target=read_a_little, daemon=True)
        reader.start()
        reader.join(timeout=30)

        assert finished.is_set(), "closing the read must not hang on parked workers"


class TestStateFollowsTheRecords:
    """A checkpoint can be taken between any two records the consumer emits, so
    state that claims a repository before its records have left would lose them
    to a crash in that window — and the idle gate would skip it next sync."""

    def test_state_is_not_published_while_records_are_still_queued(self):
        repos = fleet_in_one_bucket(1)
        repo = repos[0]

        class ThreeCommits(FleetClient):
            def commits_between(self, repo, include, exclude):
                return iter([{"hash": f"c{index}", "date": DATE} for index in range(3)])

        stream = build(repos, ThreeCommits(repos), 4)
        records = stream.read_records(None, stream_slice={"bucket_id": 0})

        next(records)
        assert stream.state["repositories"] == {}, (
            "the worker finished fetching, but its records have not been emitted yet"
        )

        assert len(list(records)) == 2
        assert repo_state_key(repo) in stream.state["repositories"], "and state lands once they have"

    def test_every_repository_still_checkpoints_by_the_end(self):
        repos = fleet()
        client = FleetClient(repos)
        stream = build(repos, client, 4)

        read_all_buckets(stream)

        assert set(stream.state["repositories"]) == {repo_state_key(repo) for repo in repos}

    def test_an_abandoned_read_publishes_no_state(self):
        repos = fleet_in_one_bucket(2)
        overflow = RECORD_BUFFER * 2

        class WideClient(FleetClient):
            def commits_between(self, repo, include, exclude):
                return iter([{"hash": f"{repo.slug}-{index}", "date": DATE} for index in range(overflow)])

        stream = build(repos, WideClient(repos), 2)
        records = stream.read_records(None, stream_slice={"bucket_id": 0})
        next(records)
        records.close()

        assert stream.state["repositories"] == {}, "nothing drained, nothing claimed"


class TestOutputRotatesBetweenRepositories:
    def test_one_fast_producer_does_not_hold_the_floor(self):
        """A producer that refills between yields would otherwise emit its whole
        repository first, leaving every other worker parked on a full buffer."""
        repos = fleet_in_one_bucket(2)
        history = 1_500

        class FastClient(FleetClient):
            def commits_between(self, repo, include, exclude):
                return iter([{"hash": f"{repo.slug}-{index}", "date": DATE} for index in range(history)])

        stream = build(repos, FastClient(repos), 2)
        opening: list[str] = []

        # A fast consumer empties each queue before the producer refills, testing nothing.
        # Closed explicitly: this test fails while holding the generator, which parks workers.
        with closing(stream.read_records(None, stream_slice={"bucket_id": 0})) as records:
            for record in records:
                opening.append(record["repo_slug"])
                time.sleep(0.0001)
                if len(opening) == history // 2:
                    break

        assert len(set(opening)) == 2, (
            f"the first {len(opening)} records all came from {opening[0]}; output must rotate"
        )


class TestASlowConsumerIsNotAnAbsentOne:
    def test_a_paused_consumer_still_receives_every_record(self):
        """A full queue means the destination is slow, not that the read was
        abandoned: the worker parks and delivers once consumption resumes."""
        repos = fleet_in_one_bucket(1)
        history = RECORD_BUFFER * 3

        class WideClient(FleetClient):
            def commits_between(self, repo, include, exclude):
                return iter([{"hash": f"c{index}", "date": DATE} for index in range(history)])

        stream = build(repos, WideClient(repos), 2)
        seen = 0

        with closing(stream.read_records(None, stream_slice={"bucket_id": 0})) as records:
            for record in records:
                assert record["record_type"] == "item"
                if seen == 0:
                    # Long enough for the worker to fill the buffer and park.
                    time.sleep(QUEUE_POLL_SECONDS * 3)
                seen += 1

        assert seen == history, f"parked worker delivered {seen} of {history}"
        assert repo_state_key(repos[0]) in stream.state["repositories"]
