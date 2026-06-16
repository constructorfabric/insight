from __future__ import annotations

from collections.abc import Iterable, Mapping, MutableMapping
from typing import Any

from airbyte_cdk.models import AirbyteMessage, SyncMode
from airbyte_cdk.sources.streams import IncrementalMixin

from source_gitlab.streams.base import GitlabStream, GitlabSubstream, subtract_minutes
from source_gitlab.streams.windowing import UpdatedAtWindowing

CURSOR_OVERLAP_MINUTES = 1


class ProjectUpdatedAtStream(UpdatedAtWindowing, GitlabSubstream, IncrementalMixin):
    cursor_field = "updated_at"

    def __init__(self, *, parent: GitlabStream, **kwargs: Any) -> None:
        super().__init__(parent=parent, **kwargs)
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
                yield {"project_id": project_id, "updated_after": updated_after}

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any] | AirbyteMessage]:
        project_id = (stream_slice or {}).get("project_id")
        latest = (
            self._state.get("projects", {}).get(str(project_id), {}).get("updated_at")
            if project_id is not None
            else None
        )
        for record in self._windowed_records(
            sync_mode,
            cursor_field=cursor_field,
            stream_slice=stream_slice,
            stream_state=stream_state,
        ):
            if isinstance(record, Mapping) and record.get("updated_at"):
                latest = record["updated_at"]
            yield record
        if project_id is not None and latest:
            self._project_state(project_id)["updated_at"] = latest

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
