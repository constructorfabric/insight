from __future__ import annotations

import hashlib
import json
import logging
import queue
import re
import threading
import uuid
from abc import ABC
from collections import deque
from collections.abc import Iterable, Mapping, MutableMapping, Sequence
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
from typing import Any, NamedTuple

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams import CheckpointMixin, Stream

from source_bitbucket_cloud.client import (
    DENIED_STATUSES,
    BitbucketApiError,
    BitbucketClient,
    RepositoryCatalog,
    RepositoryRef,
)

logger = logging.getLogger("airbyte")

BUCKET_COUNT = 8
# Commit messages, PR descriptions and comment bodies are kept for display,
# not parsed: nothing downstream reads past the opening lines, while generated
# descriptions routinely run to tens of KB and multiply bronze storage.
MAX_TEXT_BYTES = 2_048
# Bumped from 2 when entity and state keys moved back to workspace/slug: a
# version-2 state is keyed by repository uuid and no longer addresses anything.
STATE_VERSION = 3
# Records a worker may run ahead of the consumer, per repository. Bounded so a
# repository with a long history cannot buffer itself into memory.
RECORD_BUFFER = 500
# Records taken from one repository before moving to the next. A producer that
# refills faster than the consumer emits would otherwise hold the floor until
# its repository finished, leaving every other worker parked on a full buffer.
DRAIN_BATCH = 64
QUEUE_POLL_SECONDS = 0.5
DEFAULT_CONCURRENCY = 4
MAX_CONCURRENCY = 16
_READ_DONE = object()


class _PendingRead(NamedTuple):
    repo: RepositoryRef
    records: queue.Queue[Any]


class _StagedState(NamedTuple):
    """State a worker produced, travelling behind that worker's records.

    A checkpoint may be taken between any two records the consumer emits, so
    state must only claim a repository once its records have actually left —
    otherwise a crash in that window loses them and the idle gate skips the
    repository on the next sync.
    """

    entries: Mapping[str, Mapping[str, Any]]


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def normalize_start_date(value: str | None) -> str | None:
    if not value:
        return None
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as exc:
        raise ValueError(
            f"bitbucket_start_date must be an ISO date (YYYY-MM-DD), got {value!r}"
        ) from exc
    return parsed.date().isoformat()


def truncate(value: Any, limit: int = MAX_TEXT_BYTES) -> str | None:
    if value is None:
        return None
    encoded = str(value).encode("utf-8", errors="replace")
    if len(encoded) <= limit:
        return str(value)
    return encoded[:limit].decode("utf-8", errors="ignore")


def unique_key(tenant_id: str, source_id: str, *parts: Any) -> str:
    encoded = [str(part).replace(":", "%3A") for part in parts]
    return ":".join([tenant_id, source_id, *encoded])


def repo_scope(repo: RepositoryRef) -> tuple[str, str]:
    """Identity parts for an entity key: workspace + slug.

    These are the parts the pre-rewrite connector used, so a re-synced entity
    lands on the same `unique_key` as the row that connector already wrote and
    supersedes it in place instead of duplicating it.
    """
    return (repo.workspace, repo.slug)


def repo_state_key(repo: RepositoryRef) -> str:
    """State-dict key for a repository, matching the pre-rewrite partition key."""
    return f"{repo.workspace}/{repo.slug}"


def repository_bucket(state_key: str) -> int:
    digest = hashlib.sha256(state_key.encode("utf-8")).digest()
    return int.from_bytes(digest[:4], "big") % BUCKET_COUNT


def migrate_legacy_state(value: Mapping[str, Any]) -> dict[str, Any]:
    """Fold a pre-rewrite state dict into the per-repository shape.

    The old connector kept one flat entry per partition, and the partition was a
    branch (`ws/slug/branch` carrying `head_sha`) or a pull request
    (`ws/slug/pr_id` carrying `pull_request_updated_on`) or the repository itself
    (`ws/slug` carrying `updated_on`). Everything the new streams need is already
    in there — the branch heads are exactly the `exclude` set of the commit-range
    diff, and the max cursor is the pull-request watermark — so this reshapes
    rather than reconstructs, and an upgraded connector resumes from the old
    checkpoints instead of re-reading history.

    A partition that cannot be parsed is skipped: the repository then looks
    unsynced and is fetched in full, which is safe because re-fetched rows
    supersede the old ones on the same key.
    """
    repositories: dict[str, dict[str, Any]] = {}
    for partition, entry in value.items():
        if not isinstance(entry, Mapping):
            continue
        segments = str(partition).split("/")
        if len(segments) < 2:
            continue
        key = "/".join(segments[:2])
        repository = repositories.setdefault(key, {})
        head_sha = entry.get("head_sha")
        if head_sha:
            repository.setdefault("head_shas", set()).add(str(head_sha))
        cursor = entry.get("updated_on") or entry.get("pull_request_updated_on")
        if cursor:
            repository["updated_on"] = max(str(cursor), repository.get("updated_on", ""))
    for repository in repositories.values():
        if "head_shas" in repository:
            repository["head_shas"] = sorted(repository["head_shas"])
        if "updated_on" in repository:
            repository["reconcile_after_id"] = 0
    return {"version": STATE_VERSION, "bucket_count": BUCKET_COUNT, "repositories": repositories}


def schema(properties: Mapping[str, Any], *, additional: bool = False) -> Mapping[str, Any]:
    base = {
        "tenant_id": {"type": "string"},
        "source_id": {"type": "string"},
        "unique_key": {"type": "string"},
        "entity_key": {"type": ["null", "string"]},
        "data_source": {"type": "string"},
        "collected_at": {"type": "string"},
        "record_type": {"type": ["null", "string"]},
        "generation_id": {"type": ["null", "string"]},
        "bucket_id": {"type": ["null", "integer"]},
        "snapshot_item_count": {"type": ["null", "integer"]},
        "snapshot_available": {"type": ["null", "boolean"]},
        "repository_uuid": {"type": ["null", "string"]},
        "workspace_uuid": {"type": ["null", "string"]},
    }
    return {
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "additionalProperties": additional,
        "properties": {**base, **properties},
    }


class BitbucketStream(Stream, ABC):
    primary_key = "unique_key"
    data_source = "insight_bitbucket_cloud"
    state_checkpoint_interval = None

    def __init__(
        self,
        *,
        token: str,
        tenant_id: str,
        source_id: str,
        workspaces: Sequence[str],
        username: str = "",
        skip_forks: bool = True,
        start_date: str | None = None,
        concurrency: int = 1,
        client: BitbucketClient | None = None,
        catalog: RepositoryCatalog | None = None,
    ) -> None:
        self._client = client or BitbucketClient(token, username)
        self._concurrency = max(1, min(MAX_CONCURRENCY, concurrency))
        self._tenant_id = tenant_id
        self._source_id = source_id
        self._workspaces = tuple(workspaces)
        self._skip_forks = skip_forks
        self._start_date = normalize_start_date(start_date)
        self._run_id = uuid.uuid4().hex
        self._catalog = catalog or RepositoryCatalog(self._client, self._workspaces, self._skip_forks)
        self._repositories_by_bucket: dict[int, list[RepositoryRef]] = {}
        self._failed_repositories: list[str] = []
        self._skipped_repositories: list[str] = []

    def stream_slices(
        self,
        *,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any]]:
        del sync_mode, cursor_field, stream_state
        self._bucket_repositories()
        for bucket in range(BUCKET_COUNT):
            yield {"bucket_id": bucket}

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any]]:
        del sync_mode, cursor_field, stream_state
        bucket_id, repositories = self.bucket(stream_slice)
        read = self._read_serially if self._concurrency <= 1 else self._read_concurrently
        yield from read(bucket_id, repositories)
        self.finish_bucket(bucket_id, repositories)

    def _read_serially(
        self, bucket_id: int, repositories: Sequence[RepositoryRef]
    ) -> Iterable[Mapping[str, Any]]:
        for repo in repositories:
            if self.already_denied(repo):
                continue
            try:
                yield from self.repository_records(repo, bucket_id)
            except BaseException as error:  # noqa: BLE001 - classified below
                self.handle_repository_error(repo, error)

    def _read_concurrently(
        self, bucket_id: int, repositories: Sequence[RepositoryRef]
    ) -> Iterable[Mapping[str, Any]]:
        """Read several repositories at once, emit whatever is ready.

        Fetching is the whole cost of a bucket and the per-repository reads are
        independent. Consuming in submission order would let one repository
        with a deep history park every other worker on a full buffer and block
        new submissions behind it; records interleave across repositories
        instead, which bronze does not mind (append-only, keyed). Failures stay
        attributed: each repository has its own queue, and its error travels on
        it.
        """
        stop = threading.Event()
        pending: deque[_PendingRead] = deque()
        waiting = iter(repositories)
        with ThreadPoolExecutor(max_workers=self._concurrency) as pool:
            # Inside the pool: leaving the sync early (the consumer stops
            # reading, or a worker raises) must release the workers parked on a
            # full buffer before shutdown waits for them.
            try:
                while True:
                    while len(pending) < self._concurrency:
                        repo = next(waiting, None)
                        if repo is None:
                            break
                        if self.already_denied(repo):
                            continue
                        pending.append(self._submit(pool, repo, bucket_id, stop))
                    if not pending:
                        return
                    yield from self._drain_ready(pending)
            finally:
                stop.set()

    def _drain_ready(self, pending: deque[_PendingRead]) -> Iterable[Mapping[str, Any]]:
        drained_any = False
        for read in list(pending):
            for _ in range(DRAIN_BATCH):
                try:
                    item = read.records.get_nowait()
                except queue.Empty:
                    break
                drained_any = True
                if item is _READ_DONE:
                    pending.remove(read)
                    break
                if isinstance(item, _StagedState):
                    self.apply_staged_state(item.entries)
                    continue
                if isinstance(item, BaseException):
                    self.handle_repository_error(read.repo, item)
                    continue
                yield item

        if drained_any or not pending:
            return
        # Nothing ready anywhere: block briefly on the oldest read rather than
        # spinning; the sweep resumes with whatever else arrived meanwhile.
        oldest = pending[0]
        try:
            item = oldest.records.get(timeout=QUEUE_POLL_SECONDS)
        except queue.Empty:
            return
        if item is _READ_DONE:
            pending.remove(oldest)
        elif isinstance(item, _StagedState):
            self.apply_staged_state(item.entries)
        elif isinstance(item, BaseException):
            self.handle_repository_error(oldest.repo, item)
        else:
            yield item

    def _submit(
        self, pool: ThreadPoolExecutor, repo: RepositoryRef, bucket_id: int, stop: threading.Event
    ) -> _PendingRead:
        records: queue.Queue[Any] = queue.Queue(maxsize=RECORD_BUFFER)
        pool.submit(self._collect, repo, bucket_id, records, stop)
        return _PendingRead(repo, records)

    def _collect(
        self, repo: RepositoryRef, bucket_id: int, records: queue.Queue[Any], stop: threading.Event
    ) -> None:
        staged = self.stage_state()
        try:
            for record in self.repository_records(repo, bucket_id):
                if not self._offer(records, record, stop):
                    return
        except BaseException as error:  # noqa: BLE001 - re-raised in the consumer
            self._offer(records, error, stop)
        finally:
            self.stop_staging()
            if staged:
                self._offer(records, _StagedState(staged), stop)
            self._offer(records, _READ_DONE, stop)

    def stage_state(self) -> MutableMapping[str, Mapping[str, Any]]:
        """Redirect this worker's state commits into a buffer it owns."""
        return {}

    def stop_staging(self) -> None:
        return

    def apply_staged_state(self, entries: Mapping[str, Mapping[str, Any]]) -> None:
        del entries

    @staticmethod
    def _offer(records: queue.Queue[Any], item: Any, stop: threading.Event) -> bool:
        """Park on a full buffer for as long as the read is still wanted.

        No timeout: a full queue cannot tell an absent consumer from a slow
        destination, and giving up on the second would abandon a repository
        mid-read. `stop` belongs to the consumer, which sets it once it is
        finished with the bucket — by which point nothing waits on these
        records.
        """
        while not stop.is_set():
            try:
                records.put(item, timeout=QUEUE_POLL_SECONDS)
            except queue.Full:
                continue
            return True
        return False

    def already_denied(self, repo: RepositoryRef) -> bool:
        # Discovered by an earlier stream; still counts toward THIS stream's
        # end-of-sync skipped summary.
        if not self._catalog.is_inaccessible(repo):
            return False
        self._skipped_repositories.append(f"{repo.workspace}/{repo.slug}")
        return True

    def handle_repository_error(self, repo: RepositoryRef, error: BaseException) -> None:
        if isinstance(error, BitbucketApiError):
            if error.status_code == 401:
                # Credential failure is global, not per-repository: every
                # remaining repo would fail identically, drowning the log in
                # quarantine noise before a generic end-of-sync error. Abort
                # now with the actionable cause instead.
                raise RuntimeError(
                    "Bitbucket authentication failed mid-sync (HTTP 401): the token was "
                    "rejected. If bitbucket_username is unset, Atlassian API tokens are "
                    "sent as Bearer and refused — set the username, or the token has "
                    "expired/been rotated."
                ) from error
            if error.status_code in DENIED_STATUSES:
                self.skip_repository(repo, error.status_code)
                return
        if not isinstance(error, Exception):
            raise error
        self.record_failure(repo, error)

    def repository_records(self, repo: RepositoryRef, bucket_id: int) -> Iterable[Mapping[str, Any]]:
        raise NotImplementedError

    def before_start_date(self, timestamp: Any) -> bool:
        return bool(self._start_date and timestamp and str(timestamp)[:10] < self._start_date)

    def out_of_window(self, repo: RepositoryRef) -> bool:
        """True when nothing a push produces can fall inside the start window.

        Commits, branches and tags only appear by being pushed, and a push
        moves the repository's updated_on, so a repository last touched before
        start_date holds nothing this sync is asked for.
        """
        updated_on = str(repo.raw.get("updated_on") or "")
        return bool(self._start_date and updated_on and updated_on[:10] < self._start_date)

    def bucket(self, stream_slice: Mapping[str, Any] | None) -> tuple[int, list[RepositoryRef]]:
        bucket_id = int((stream_slice or {}).get("bucket_id", 0))
        return bucket_id, self.repositories_for_slice(stream_slice)

    def record_failure(self, repo: RepositoryRef, error: BaseException | None = None) -> None:
        """A failure worth surfacing: transient, so retrying the sync may fix it."""
        name = f"{repo.workspace}/{repo.slug}"
        self._failed_repositories.append(name)
        logger.error(
            f"{self.name}: repository {name} failed; its state was not advanced, continuing",
            exc_info=error or True,
        )

    def skip_repository(self, repo: RepositoryRef, status_code: int) -> None:
        """A repository the token cannot read: skip it without failing the sync.

        A repository can be listed for the workspace and still deny every request
        under it, which is routine with repo-scoped tokens and per-repository
        permissions. That is a configuration fact, not an incident: retrying will
        never change it, so counting it as a failure would leave the sync red
        forever and bury the transient failures that do deserve attention. The
        repository is marked on the shared catalog so the remaining streams skip
        it instead of each rediscovering the same 403.
        """
        name = f"{repo.workspace}/{repo.slug}"
        already_known = self._catalog.is_inaccessible(repo)
        self._catalog.mark_inaccessible(repo)
        if not already_known:
            logger.warning(
                f"{self.name}: repository {name} denied access (HTTP {status_code}); "
                "skipping it for the rest of this sync"
            )
        self._skipped_repositories.append(name)

    def finish_bucket(self, bucket_id: int, repositories: Sequence[RepositoryRef]) -> None:
        del repositories
        if bucket_id != BUCKET_COUNT - 1:
            return

        cached_repositories, cached_branches = self._catalog.branch_cache_size
        logger.info(f"{self.name}: branch_cache repositories={cached_repositories} branches={cached_branches}")

        if self._skipped_repositories:
            logger.info(
                f"{self.name}: skipped {len(self._skipped_repositories)} inaccessible "
                f"repositories: {', '.join(sorted(set(self._skipped_repositories))[:10])}"
            )
        if self._failed_repositories:
            raise RuntimeError(
                f"{self.name}: {len(self._failed_repositories)} repositories failed this sync: "
                + ", ".join(self._failed_repositories[:10])
            )

    def repositories_for_slice(self, stream_slice: Mapping[str, Any] | None) -> list[RepositoryRef]:
        bucket = int((stream_slice or {}).get("bucket_id", 0))
        if not self._repositories_by_bucket:
            self._bucket_repositories()
        return self._repositories_by_bucket[bucket]

    def _bucket_repositories(self) -> None:
        self._repositories_by_bucket = {bucket: [] for bucket in range(BUCKET_COUNT)}
        for repo in self._catalog.repositories():
            self._repositories_by_bucket[repository_bucket(repo_state_key(repo))].append(repo)

    def envelope(self, record: Mapping[str, Any]) -> dict[str, Any]:
        return {
            **record,
            "tenant_id": self._tenant_id,
            "source_id": self._source_id,
            "data_source": self.data_source,
            "collected_at": now_iso(),
        }

    def item(self, *, entity_key: str, generation_id: str | None = None, **record: Any) -> dict[str, Any]:
        storage_key = f"{entity_key}:{generation_id}" if generation_id else entity_key
        return self.envelope(
            {
                **record,
                "unique_key": storage_key,
                "entity_key": entity_key,
                "record_type": "item",
                "generation_id": generation_id,
            }
        )

    def complete(
        self,
        *,
        scope_parts: Sequence[Any],
        generation_id: str,
        item_count: int,
        bucket_id: int | None = None,
        available: bool = True,
        **record: Any,
    ) -> dict[str, Any]:
        return self.envelope(
            {
                **record,
                "unique_key": unique_key(
                    self._tenant_id, self._source_id, *scope_parts, "snapshot_complete", generation_id
                ),
                "entity_key": None,
                "record_type": "snapshot_complete",
                "generation_id": generation_id,
                "bucket_id": bucket_id,
                "snapshot_item_count": item_count,
                "snapshot_available": available,
            }
        )

    def generation(self, *parts: Any) -> str:
        value = ":".join([self._run_id, *(str(part) for part in parts)])
        return hashlib.sha256(value.encode("utf-8")).hexdigest()


class BitbucketIncrementalStream(BitbucketStream, CheckpointMixin, ABC):
    # Emit state mid-bucket too: a bucket spans a large share of the fleet, and
    # a pod that dies between bucket boundaries would otherwise re-read hours
    # of finished repositories. The interval is coarse because each message
    # carries the whole repositories map.
    state_checkpoint_interval = 25_000

    def __init__(self, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        # Versioned from the start: state emitted without one reads back as
        # pre-rewrite state and gets reshaped into something addressing nothing.
        self._state: MutableMapping[str, Any] = self._empty_state()
        self._state_lock = threading.Lock()
        self._staging = threading.local()

    @property
    def state(self) -> MutableMapping[str, Any]:
        # A snapshot, not the live dict: the platform serialises this while
        # workers commit repositories. Only finished repositories are ever in
        # the map (each is committed whole), so any snapshot is a valid resume
        # point.
        with self._state_lock:
            return {**self._state, "repositories": dict(self._state.get("repositories") or {})}

    @state.setter
    def state(self, value: MutableMapping[str, Any]) -> None:
        if not value:
            self._state = self._empty_state()
        elif value.get("version") == STATE_VERSION:
            # bucket_count is not part of the address: keys are repository
            # scoped and the bucket is derived by hash at read time, so state
            # written under any bucket count resumes under any other.
            self._state = {**value, "bucket_count": BUCKET_COUNT}
        elif "version" not in value:
            # Pre-rewrite state: a flat partition -> cursor map. Reshape it so
            # the sync resumes from those checkpoints (see migrate_legacy_state).
            self._state = migrate_legacy_state(value)
            logger.info(
                f"{self.name}: migrated pre-rewrite state — "
                f"{len(self._state['repositories'])} repositories resume from their last checkpoint"
            )
        else:
            # A version we do not recognise (e.g. the uuid-keyed version 2)
            # addresses nothing here; start clean rather than half-resume.
            self._state = self._empty_state()

    @staticmethod
    def _empty_state() -> dict[str, Any]:
        return {"version": STATE_VERSION, "bucket_count": BUCKET_COUNT, "repositories": {}}

    def repository_state(self, repo: RepositoryRef) -> MutableMapping[str, Any]:
        with self._state_lock:
            repositories = self._state.setdefault("repositories", {})
            return dict(repositories.get(repo_state_key(repo)) or {})

    def commit_repository_state(self, repo: RepositoryRef, value: Mapping[str, Any]) -> None:
        staged = getattr(self._staging, "entries", None)
        if staged is not None:
            staged[repo_state_key(repo)] = dict(value)
            return
        with self._state_lock:
            self._state.setdefault("repositories", {})[repo_state_key(repo)] = dict(value)

    def stage_state(self) -> MutableMapping[str, Mapping[str, Any]]:
        self._staging.entries = {}
        return self._staging.entries

    def stop_staging(self) -> None:
        self._staging.entries = None

    def apply_staged_state(self, entries: Mapping[str, Mapping[str, Any]]) -> None:
        with self._state_lock:
            self._state.setdefault("repositories", {}).update(entries)

    def prune_bucket_state(self, bucket_id: int, repositories: Sequence[RepositoryRef]) -> None:
        current = {repo_state_key(repo) for repo in repositories}

        with self._state_lock:
            state_repositories = self._state.setdefault("repositories", {})
            stale = [key for key in state_repositories if repository_bucket(key) == bucket_id and key not in current]
            for key in stale:
                del state_repositories[key]

    def finish_bucket(self, bucket_id: int, repositories: Sequence[RepositoryRef]) -> None:
        self.prune_bucket_state(bucket_id, repositories)
        self.log_state_size()
        super().finish_bucket(bucket_id, repositories)

    def log_state_size(self) -> None:
        snapshot = self.state
        encoded = json.dumps(snapshot, separators=(",", ":")).encode("utf-8")
        logger.info(
            f"{self.name}: state_repositories={len(snapshot.get('repositories', {}))} state_bytes={len(encoded)}"
        )


AUTHOR_RE = re.compile(r"^(.*?)\s*<([^>]+)>\s*$")
