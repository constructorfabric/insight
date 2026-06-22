from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from airbyte_cdk.models import AirbyteMessage, SyncMode

from source_gitlab.streams import concurrency
from source_gitlab.streams.base import GitlabStream, GitlabSubstream
from source_gitlab.streams.concurrency import RequestGate


class BranchesStream(GitlabSubstream):
    name = "branches"

    def __init__(self, *, parent: GitlabStream, gate: RequestGate, **kwargs: Any) -> None:
        super().__init__(parent=parent, **kwargs)
        self._gate = gate

    def stream_slices(self, **kwargs: Any) -> Iterable[Mapping[str, Any] | None]:
        yield {}

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any] | AirbyteMessage]:
        slice_ = stream_slice or {}
        if "parent" in slice_:
            yield from self._project_branches(slice_["parent"])
            return
        for records in concurrency.imap_bounded(
            self._gate, self._iter_unique_projects(), self._branches_task
        ):
            yield from records

    def _branches_task(self, project: Mapping[str, Any]) -> list[Mapping[str, Any]]:
        return list(self._project_branches(project))

    def _project_branches(
        self, project: Mapping[str, Any]
    ) -> Iterable[Mapping[str, Any]]:
        slice_ = {"parent": project}
        yield from concurrency.paginate(
            self._gate,
            url_base=self.url_base,
            path=self._path(stream_slice=slice_),
            params=self._initial_params(slice_),
            envelope_fn=lambda raw: self._envelope(raw, slice_),
            headers={"PRIVATE-TOKEN": self._token},
            skippable=self.skippable_statuses,
        )

    def _path(self, *, stream_slice: Mapping[str, Any] | None) -> str:
        project_id = self._parent_value(stream_slice, "id")
        return f"projects/{project_id}/repository/branches"

    def _initial_params(
        self, stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        return {"per_page": self.page_size}

    def _record_key(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> list[str]:
        project_id = (stream_slice or {})["parent"]["id"]
        return [str(project_id), str(record["name"])]

    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        project_id = (stream_slice or {})["parent"]["id"]
        commit = record.get("commit") or {}
        return {
            "project_id": project_id,
            "name": record.get("name"),
            "commit_sha": commit.get("id"),
            "default": record.get("default"),
            "protected": record.get("protected"),
            "merged": record.get("merged"),
        }
