from __future__ import annotations

import hashlib
import json
import logging
import re
import uuid
from abc import ABC
from collections.abc import Iterable, Mapping, MutableMapping, Sequence
from datetime import datetime, timezone
from typing import Any

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams import CheckpointMixin, Stream

from source_bitbucket_cloud.client import BitbucketClient, RepositoryCatalog, RepositoryRef

logger = logging.getLogger("airbyte")

BUCKET_COUNT = 8
MAX_TEXT_BYTES = 16_384


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


def repository_bucket(repository_uuid: str) -> int:
    digest = hashlib.sha256(repository_uuid.encode("utf-8")).digest()
    return int.from_bytes(digest[:4], "big") % BUCKET_COUNT


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
        client: BitbucketClient | None = None,
        catalog: RepositoryCatalog | None = None,
    ) -> None:
        self._client = client or BitbucketClient(token, username)
        self._tenant_id = tenant_id
        self._source_id = source_id
        self._workspaces = tuple(workspaces)
        self._skip_forks = skip_forks
        self._start_date = normalize_start_date(start_date)
        self._run_id = uuid.uuid4().hex
        self._catalog = catalog or RepositoryCatalog(self._client, self._workspaces, self._skip_forks)
        self._repositories_by_bucket: dict[int, list[RepositoryRef]] = {}
        self._failed_repositories: list[str] = []

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
        for repo in repositories:
            try:
                yield from self.repository_records(repo, bucket_id)
            except Exception:
                self.record_failure(repo)
        self.finish_bucket(bucket_id, repositories)

    def repository_records(self, repo: RepositoryRef, bucket_id: int) -> Iterable[Mapping[str, Any]]:
        raise NotImplementedError

    def bucket(self, stream_slice: Mapping[str, Any] | None) -> tuple[int, list[RepositoryRef]]:
        bucket_id = int((stream_slice or {}).get("bucket_id", 0))
        return bucket_id, self.repositories_for_slice(stream_slice)

    def record_failure(self, repo: RepositoryRef) -> None:
        name = f"{repo.workspace}/{repo.slug}"
        self._failed_repositories.append(name)
        logger.exception(f"{self.name}: repository {name} failed; its state was not advanced, continuing")

    def finish_bucket(self, bucket_id: int, repositories: Sequence[RepositoryRef]) -> None:
        del repositories
        if bucket_id == BUCKET_COUNT - 1 and self._failed_repositories:
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
            self._repositories_by_bucket[repository_bucket(repo.uuid)].append(repo)

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
    def __init__(self, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self._state: MutableMapping[str, Any] = {}

    @property
    def state(self) -> MutableMapping[str, Any]:
        return self._state

    @state.setter
    def state(self, value: MutableMapping[str, Any]) -> None:
        if value and value.get("version") == 2 and value.get("bucket_count") == BUCKET_COUNT:
            self._state = value
        else:
            self._state = {"version": 2, "bucket_count": BUCKET_COUNT, "repositories": {}}

    def repository_state(self, repo: RepositoryRef) -> MutableMapping[str, Any]:
        repositories = self._state.setdefault("repositories", {})
        return dict(repositories.get(repo.uuid) or {})

    def commit_repository_state(self, repo: RepositoryRef, value: Mapping[str, Any]) -> None:
        self._state.setdefault("repositories", {})[repo.uuid] = dict(value)

    def prune_bucket_state(self, bucket_id: int, repositories: Sequence[RepositoryRef]) -> None:
        current = {repo.uuid for repo in repositories}
        state_repositories = self._state.setdefault("repositories", {})
        stale = [key for key in state_repositories if repository_bucket(key) == bucket_id and key not in current]
        for key in stale:
            del state_repositories[key]

    def finish_bucket(self, bucket_id: int, repositories: Sequence[RepositoryRef]) -> None:
        self.prune_bucket_state(bucket_id, repositories)
        self.log_state_size()
        super().finish_bucket(bucket_id, repositories)

    def log_state_size(self) -> None:
        encoded = json.dumps(self._state, separators=(",", ":")).encode("utf-8")
        logger.info(
            f"{self.name}: state_repositories={len(self._state.get('repositories', {}))} state_bytes={len(encoded)}"
        )


AUTHOR_RE = re.compile(r"^(.*?)\s*<([^>]+)>\s*$")
