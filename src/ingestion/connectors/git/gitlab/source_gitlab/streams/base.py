from __future__ import annotations

import json
from abc import ABC, abstractmethod
from collections.abc import Iterable, Mapping, MutableMapping
from datetime import datetime, timezone
from functools import cache
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

import requests
from airbyte_cdk.models import AirbyteMessage, SyncMode
from airbyte_cdk.sources.streams.http import HttpStream

from source_gitlab.streams import concurrency
from source_gitlab.streams.errors import GitlabApiError, GitlabAuthError

_API_PREFIX = "/api/v4/"

MAX_BODY_CHARS = 16384
MAX_TITLE_CHARS = 1024


def _now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def trim_text(value: Any, limit: int) -> tuple[str | None, bool]:
    if value is None:
        return None, False
    text = str(value)
    if len(text) <= limit:
        return text, False
    return text[:limit], True


MAX_DIFF_CHARS = 1_000_000


def parse_diff_counts(diff_obj: Mapping[str, Any]) -> tuple[int | None, int | None, bool]:
    if diff_obj.get("too_large") or diff_obj.get("collapsed"):
        return None, None, True
    text = diff_obj.get("diff") or ""
    if not text:
        return 0, 0, False
    if len(text) > MAX_DIFF_CHARS:
        return None, None, True
    added = removed = 0
    in_hunk = False
    for line in text.split("\n"):
        if line.startswith("@@"):
            in_hunk = True
            continue
        if not in_hunk:
            continue
        if line.startswith("+"):
            added += 1
        elif line.startswith("-"):
            removed += 1
    return added, removed, False


class GitlabStream(HttpStream, ABC):
    primary_key = "unique_key"
    raise_on_http_errors = False
    page_size = 100
    data_source = "insight_gitlab"
    skippable_statuses: frozenset[int] = frozenset()

    def __init__(
        self,
        *,
        base_url: str,
        token: str,
        tenant_id: str,
        source_id: str,
        **kwargs: Any,
    ) -> None:
        super().__init__(**kwargs)
        self._base_url = base_url
        self._token = token
        self._tenant_id = tenant_id
        self._source_id = source_id

    @property
    def url_base(self) -> str:
        return f"{self._base_url}/api/v4/"

    def request_headers(
        self,
        stream_state: Mapping[str, Any] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        next_page_token: Mapping[str, Any] | None = None,
    ) -> Mapping[str, Any]:
        return {"PRIVATE-TOKEN": self._token}

    def next_page_token(self, response: requests.Response) -> Mapping[str, Any] | None:
        nxt = response.links.get("next")
        if nxt and nxt.get("url"):
            return {"next_url": nxt["url"]}
        return None

    def path(
        self,
        *,
        stream_state: Mapping[str, Any] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        next_page_token: Mapping[str, Any] | None = None,
    ) -> str:
        if next_page_token and "next_url" in next_page_token:
            return self._relative_next_path(str(next_page_token["next_url"]))
        return self._path(stream_slice=stream_slice)

    def _relative_next_path(self, next_url: str) -> str:
        split = urlsplit(next_url)
        path = split.path
        marker = path.find(_API_PREFIX)
        relative = path[marker + len(_API_PREFIX) :] if marker != -1 else path.lstrip("/")
        return f"{relative}?{split.query}" if split.query else relative

    def request_params(
        self,
        stream_state: Mapping[str, Any] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        next_page_token: Mapping[str, Any] | None = None,
    ) -> MutableMapping[str, Any]:
        if next_page_token:
            return {}
        return dict(self._initial_params(stream_slice))

    def should_retry(self, response: requests.Response) -> bool:
        if not isinstance(response, requests.Response):
            return True
        return concurrency.should_retry(response)

    def backoff_time(self, response: requests.Response) -> float | None:
        if not isinstance(response, requests.Response):
            return 60.0
        return concurrency.backoff_time(response)

    def parse_response(
        self,
        response: requests.Response,
        *,
        stream_state: Mapping[str, Any] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        next_page_token: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any]]:
        code = response.status_code
        if self._should_skip(code, stream_slice, next_page_token):
            self.logger.warning(
                f"{self.name}: HTTP {code} on {response.url} — skipping this "
                f"entity and advancing (expected for deleted / disabled / "
                f"hidden resources)"
            )
            return
        if code in (401, 403):
            raise GitlabAuthError(
                f"GitLab auth error ({code}): {response.text[:200]}"
            )
        if code >= 400:
            raise GitlabApiError(
                f"Unexpected HTTP {code} on {response.url}: {response.text[:200]}"
            )
        data = response.json()
        records = data if isinstance(data, list) else [data]
        for record in records:
            yield self._envelope(record, stream_slice)

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any] | AirbyteMessage]:
        yield from self._read_pages(
            lambda _request, response, state, slice_: self.parse_response(
                response, stream_slice=slice_, stream_state=state
            ),
            stream_slice,
            stream_state,
        )

    def _should_skip(
        self,
        code: int,
        stream_slice: Mapping[str, Any] | None,
        next_page_token: Mapping[str, Any] | None,
    ) -> bool:
        if next_page_token is not None or code not in self.skippable_statuses:
            return False
        return not (stream_slice is not None and stream_slice.get("skip_404") is False)

    @cache
    def get_json_schema(self) -> Mapping[str, Any]:
        schema_path = Path(__file__).parent / f"{self.name}.schema.json"
        schema: dict[str, Any] = json.loads(schema_path.read_text())
        return schema

    def _envelope(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> dict[str, Any]:
        out = dict(self._project(record, stream_slice))
        out["tenant_id"] = self._tenant_id
        out["source_id"] = self._source_id
        out["data_source"] = self.data_source
        out["collected_at"] = _now_iso()
        parts = self._record_key(record, stream_slice)
        out["unique_key"] = ":".join([self._tenant_id, self._source_id, *parts])
        return out

    @abstractmethod
    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]: ...

    @abstractmethod
    def _path(self, *, stream_slice: Mapping[str, Any] | None) -> str: ...

    @abstractmethod
    def _initial_params(
        self, stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]: ...

    @abstractmethod
    def _record_key(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> list[str]: ...


class ScopedGitlabStream(GitlabStream, ABC):
    def __init__(
        self,
        *,
        groups: tuple[str, ...],
        projects: tuple[str, ...],
        **kwargs: Any,
    ) -> None:
        super().__init__(**kwargs)
        self._groups = groups
        self._projects = projects

    def stream_slices(self, **kwargs: Any) -> Iterable[Mapping[str, Any] | None]:
        if not self._groups and not self._projects:
            yield {"mode": "instance"}
            return
        for group in self._groups:
            yield {"mode": "group", "group": group}
        for project in self._projects:
            yield {"mode": "project", "project": project}


class GitlabSubstream(GitlabStream, ABC):
    skippable_statuses = frozenset({404})

    def __init__(self, *, parent: GitlabStream, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self._parent = parent

    def stream_slices(self, **kwargs: Any) -> Iterable[Mapping[str, Any] | None]:
        for parent_slice in self._parent.stream_slices(sync_mode=SyncMode.full_refresh):
            for parent_record in self._parent.read_records(
                sync_mode=SyncMode.full_refresh, stream_slice=parent_slice
            ):
                yield {"parent": parent_record}

    def _parent_value(
        self, stream_slice: Mapping[str, Any] | None, field: str
    ) -> Any:
        parent = (stream_slice or {}).get("parent") or {}
        value = parent.get(field)
        if value is None:
            raise GitlabApiError(
                f"{self.name}: parent record missing routing field '{field}' "
                f"— cannot build request path (routing bug, not a deleted entity)"
            )
        return value

    def _iter_unique_projects(self) -> Iterable[Mapping[str, Any]]:
        seen: set[Any] = set()
        for parent_slice in self._parent.stream_slices(sync_mode=SyncMode.full_refresh):
            for project in self._parent.read_records(
                sync_mode=SyncMode.full_refresh, stream_slice=parent_slice
            ):
                if not isinstance(project, Mapping):
                    continue
                project_id = project.get("id")
                if project_id is None or project_id in seen:
                    continue
                seen.add(project_id)
                yield project
