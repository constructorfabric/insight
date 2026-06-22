from __future__ import annotations

from collections.abc import Iterable, Mapping, MutableMapping
from typing import Any

from airbyte_cdk.models import AirbyteMessage, SyncMode
from airbyte_cdk.sources.streams import IncrementalMixin

from source_gitlab.streams import concurrency
from source_gitlab.streams.base import GitlabStream, ScopedGitlabStream
from source_gitlab.streams.concurrency import OrderedPrefix, RequestGate
from source_gitlab.streams.scope import (
    advance_cursor,
    compute_floor,
    scope_bases,
    scope_key,
    scope_params,
    scope_path,
)
from source_gitlab.streams.windowing import UpdatedAtWindowing


class MergeRequestChildStream(ScopedGitlabStream, IncrementalMixin):
    cursor_field = "mr_updated_at"
    skippable_statuses = frozenset({404})
    state_checkpoint_interval = 1000

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
        self._enum_strategy = UpdatedAtWindowing()
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
            yield {"scope_key": scope_key(base), "base": base}

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any] | AirbyteMessage]:
        slice_ = stream_slice or {}
        key = slice_.get("scope_key")
        base = slice_.get("base")
        if key is None or base is None:
            return
        scope_state = self._scope_state(key)
        prefix = OrderedPrefix()
        for task, records in concurrency.imap_bounded(
            self._gate, self._mr_tasks(base), self._fetch_child
        ):
            yield from records
            for updated_at in prefix.complete(task["seq"], task["mr_updated_at"]):
                advance_cursor(scope_state, updated_at)

    def _mr_tasks(self, base: Mapping[str, Any]) -> Iterable[Mapping[str, Any]]:
        watermark = self._state.get("scopes", {}).get(scope_key(base), {}).get("updated_at")
        enum_scope = {**base, "updated_after": compute_floor(self._start_date, watermark)}
        for seq, mr in enumerate(self._enum_mrs(enum_scope)):
            yield {
                "seq": seq,
                "project_id": mr["project_id"],
                "mr_iid": mr["iid"],
                "mr_updated_at": mr.get("updated_at"),
            }

    def _enum_mrs(self, enum_scope: Mapping[str, Any]) -> Iterable[Mapping[str, Any]]:
        for mr in concurrency.walk_window(
            strategy=self._enum_strategy,
            base_slice=enum_scope,
            url_base=self.url_base,
            path_fn=lambda applied: scope_path("merge_requests", applied),
            params_fn=lambda applied: scope_params(self.page_size, applied),
            envelope_fn=self._enum_envelope,
            headers={"PRIVATE-TOKEN": self._token},
            gate=self._gate,
            skippable=frozenset(),
        ):
            if mr.get("iid") is not None and mr.get("project_id") is not None:
                yield mr

    def _enum_envelope(
        self, raw: Mapping[str, Any], stream_slice: Mapping[str, Any]
    ) -> Mapping[str, Any]:
        return {
            "project_id": raw.get("project_id"),
            "iid": raw.get("iid"),
            "updated_at": raw.get("updated_at"),
        }

    def _fetch_child(
        self, task: Mapping[str, Any]
    ) -> tuple[Mapping[str, Any], list[Mapping[str, Any]]]:
        records = list(
            concurrency.paginate(
                self._gate,
                url_base=self.url_base,
                path=self._path(stream_slice=task),
                params=self._initial_params(task),
                envelope_fn=lambda raw: self._envelope(raw, task),
                headers={"PRIVATE-TOKEN": self._token},
                skippable=self.skippable_statuses,
            )
        )
        return task, records

    def _initial_params(
        self, stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        return {"per_page": self.page_size}
