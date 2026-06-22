from __future__ import annotations

from collections.abc import Iterable, Mapping, MutableMapping
from typing import Any
from urllib.parse import quote

from airbyte_cdk.models import AirbyteMessage, SyncMode
from airbyte_cdk.sources.streams import IncrementalMixin

from source_gitlab.streams import concurrency
from source_gitlab.streams.base import GitlabStream, ScopedGitlabStream
from source_gitlab.streams.concurrency import RequestGate
from source_gitlab.streams.timeutil import parse_iso, subtract_minutes
from source_gitlab.streams.windowing import UpdatedAtWindowing

CURSOR_OVERLAP_MINUTES = 1


def scope_bases(
    groups: tuple[str, ...],
    projects: tuple[str, ...],
    parent: GitlabStream,
) -> Iterable[Mapping[str, Any]]:
    if not groups and not projects:
        yield {"mode": "instance"}
        return
    seen: set[Any] = set()
    for parent_slice in parent.stream_slices(sync_mode=SyncMode.full_refresh):
        for project in parent.read_records(
            sync_mode=SyncMode.full_refresh, stream_slice=parent_slice
        ):
            if not isinstance(project, Mapping):
                continue
            project_id = project.get("id")
            if project_id is None or project_id in seen:
                continue
            seen.add(project_id)
            yield {"mode": "project", "project": project_id}


def scope_key(stream_slice: Mapping[str, Any] | None) -> str:
    slice_ = stream_slice or {"mode": "instance"}
    if slice_["mode"] == "project":
        return f"project:{slice_['project']}"
    return "instance"


def scope_path(resource: str, stream_slice: Mapping[str, Any] | None) -> str:
    slice_ = stream_slice or {"mode": "instance"}
    if slice_["mode"] == "project":
        return f"projects/{quote(str(slice_['project']), safe='')}/{resource}"
    return resource


def advance_cursor(scope_state: MutableMapping[str, Any], updated_at: str | None) -> None:
    if not updated_at:
        return
    current = scope_state.get("updated_at")
    if current is None or parse_iso(updated_at) > parse_iso(current):
        scope_state["updated_at"] = updated_at


def compute_floor(start_date: str | None, watermark: str | None) -> str | None:
    if watermark:
        overlapped = subtract_minutes(watermark, CURSOR_OVERLAP_MINUTES)
        if start_date and parse_iso(start_date) > parse_iso(overlapped):
            return start_date
        return overlapped
    return start_date


def scope_params(
    page_size: int, stream_slice: Mapping[str, Any] | None
) -> dict[str, Any]:
    slice_ = stream_slice or {"mode": "instance"}
    params: dict[str, Any] = {
        "order_by": "updated_at",
        "sort": "asc",
        "per_page": page_size,
    }
    if slice_["mode"] == "instance":
        params["scope"] = "all"
    updated_after = slice_.get("updated_after")
    if updated_after:
        params["updated_after"] = updated_after
    updated_before = slice_.get("updated_before")
    if updated_before:
        params["updated_before"] = updated_before
    return params


class ScopeUpdatedAtStream(ScopedGitlabStream, IncrementalMixin):
    cursor_field = "updated_at"
    state_checkpoint_interval = 1000
    resource: str

    def __init__(
        self,
        *,
        parent: GitlabStream,
        gate: RequestGate,
        groups: tuple[str, ...],
        projects: tuple[str, ...],
        start_date: str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(groups=groups, projects=projects, **kwargs)
        self._parent = parent
        self._gate = gate
        self._start_date = start_date
        self._strategy = UpdatedAtWindowing()
        self._state: MutableMapping[str, Any] = {}

    @property
    def state(self) -> MutableMapping[str, Any]:
        return self._state

    @state.setter
    def state(self, value: MutableMapping[str, Any]) -> None:
        self._state = value or {}

    def _scope_state(self, key: str) -> MutableMapping[str, Any]:
        scopes: dict[str, Any] = self._state.setdefault("scopes", {})
        entry: dict[str, Any] = scopes.setdefault(key, {})
        return entry

    def stream_slices(self, **kwargs: Any) -> Iterable[Mapping[str, Any] | None]:
        for base in scope_bases(self._groups, self._projects, self._parent):
            key = scope_key(base)
            watermark = self._state.get("scopes", {}).get(key, {}).get("updated_at")
            yield {**base, "updated_after": compute_floor(self._start_date, watermark)}

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any] | AirbyteMessage]:
        scope_state = self._scope_state(scope_key(stream_slice))
        for record in concurrency.walk_window(
            strategy=self._strategy,
            base_slice=stream_slice or {},
            url_base=self.url_base,
            path_fn=lambda applied: self._path(stream_slice=applied),
            params_fn=self._initial_params,
            envelope_fn=self._envelope,
            headers={"PRIVATE-TOKEN": self._token},
            gate=self._gate,
            skippable=self.skippable_statuses,
        ):
            advance_cursor(scope_state, record.get("updated_at"))
            yield record

    def _path(self, *, stream_slice: Mapping[str, Any] | None) -> str:
        return scope_path(self.resource, stream_slice)

    def _initial_params(
        self, stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        return scope_params(self.page_size, stream_slice)
