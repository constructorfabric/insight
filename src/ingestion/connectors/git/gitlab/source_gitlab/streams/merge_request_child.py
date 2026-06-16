from __future__ import annotations

from collections.abc import Iterable, Mapping, MutableMapping
from functools import cache
from typing import Any

from airbyte_cdk.models import AirbyteMessage, SyncMode
from airbyte_cdk.sources.streams import IncrementalMixin

from source_gitlab.streams.base import GitlabStream, GitlabSubstream, subtract_minutes
from source_gitlab.streams.windowing import UpdatedAtWindowing

CURSOR_OVERLAP_MINUTES = 1


class _MergeRequestEnumerator(UpdatedAtWindowing, GitlabStream):
    name = "_mr_enum_internal"
    skippable_statuses = frozenset({404})

    @cache
    def get_json_schema(self) -> Mapping[str, Any]:
        return {}

    def stream_slices(self, **kwargs: Any) -> Iterable[Mapping[str, Any] | None]:
        yield None

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any] | AirbyteMessage]:
        yield from self._windowed_records(
            sync_mode, cursor_field, stream_slice, stream_state
        )

    def _path(self, *, stream_slice: Mapping[str, Any] | None) -> str:
        return f"projects/{(stream_slice or {})['project_id']}/merge_requests"

    def _initial_params(
        self, stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        params: dict[str, Any] = {
            "order_by": "updated_at",
            "sort": "asc",
            "per_page": self.page_size,
        }
        updated_after = (stream_slice or {}).get("updated_after")
        if updated_after:
            params["updated_after"] = updated_after
        updated_before = (stream_slice or {}).get("updated_before")
        if updated_before:
            params["updated_before"] = updated_before
        return params

    def _record_key(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> list[str]:
        return [str((stream_slice or {})["project_id"]), str(record["iid"])]

    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        return {"iid": record.get("iid"), "updated_at": record.get("updated_at")}


class MergeRequestChildStream(GitlabSubstream, IncrementalMixin):
    cursor_field = "mr_updated_at"

    def __init__(self, *, parent: GitlabStream, **kwargs: Any) -> None:
        super().__init__(parent=parent, **kwargs)
        self._mr_enum = _MergeRequestEnumerator(
            base_url=self._base_url,
            token=self._token,
            tenant_id=self._tenant_id,
            source_id=self._source_id,
        )
        self._state: MutableMapping[str, Any] = {}

    @property
    def state(self) -> MutableMapping[str, Any]:
        return self._state

    @state.setter
    def state(self, value: MutableMapping[str, Any]) -> None:
        self._state = value or {}

    def _project_state(self, project_id: Any) -> MutableMapping[str, Any]:
        projects: dict[str, Any] = self._state.setdefault("projects", {})
        pstate: dict[str, Any] = projects.setdefault(str(project_id), {})
        return pstate

    def stream_slices(self, **kwargs: Any) -> Iterable[Mapping[str, Any] | None]:
        for parent_slice in self._parent.stream_slices(sync_mode=SyncMode.full_refresh):
            for project in self._parent.read_records(
                sync_mode=SyncMode.full_refresh, stream_slice=parent_slice
            ):
                if not isinstance(project, Mapping):
                    continue
                project_id = project.get("id")
                if project_id is None:
                    continue
                watermark = (
                    self._state.get("projects", {}).get(str(project_id), {}).get("updated_at")
                )
                updated_after = (
                    subtract_minutes(watermark, CURSOR_OVERLAP_MINUTES) if watermark else None
                )
                enum_slice = {"project_id": project_id, "updated_after": updated_after}
                # One-item lookahead: only the final MR carries the cursor advance.
                pending: tuple[Any, Any] | None = None
                for mr in self._mr_enum.read_records(
                    sync_mode=SyncMode.full_refresh, stream_slice=enum_slice
                ):
                    if not isinstance(mr, Mapping) or mr.get("iid") is None:
                        continue
                    if pending is not None:
                        yield {
                            "project_id": project_id,
                            "mr_iid": pending[0],
                            "mr_updated_at": pending[1],
                        }
                    pending = (mr["iid"], mr.get("updated_at"))
                if pending is None:
                    continue
                yield {
                    "project_id": project_id,
                    "mr_iid": pending[0],
                    "mr_updated_at": pending[1],
                    "advance": pending[1],
                }

    def _initial_params(
        self, stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        return {"per_page": self.page_size}

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any] | AirbyteMessage]:
        yield from super().read_records(
            sync_mode,
            cursor_field=cursor_field,
            stream_slice=stream_slice,
            stream_state=stream_state,
        )
        advance = (stream_slice or {}).get("advance")
        project_id = (stream_slice or {}).get("project_id")
        if advance and project_id is not None:
            self._project_state(project_id)["updated_at"] = advance
