from __future__ import annotations

import json
from collections.abc import Iterable, Mapping, MutableMapping
from typing import Any

from airbyte_cdk.models import AirbyteMessage, SyncMode
from airbyte_cdk.sources.streams import IncrementalMixin

from source_gitlab.streams.base import (
    MAX_BODY_CHARS,
    MAX_TITLE_CHARS,
    GitlabStream,
    GitlabSubstream,
    subtract_minutes,
    trim_text,
)

CURSOR_OVERLAP_MINUTES = 1


class MergeRequestsStream(GitlabSubstream, IncrementalMixin):
    name = "merge_requests"
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
        for record in super().read_records(
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
        return params

    def _record_key(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> list[str]:
        return [str(record.get("project_id")), str(record.get("iid"))]

    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        title, title_truncated = trim_text(record.get("title"), MAX_TITLE_CHARS)
        description, description_truncated = trim_text(
            record.get("description"), MAX_BODY_CHARS
        )
        author = record.get("author") or {}
        merged_by = record.get("merged_by") or {}
        milestone = record.get("milestone") or {}
        assignees = record.get("assignees") or []
        reviewers = record.get("reviewers") or []
        return {
            "project_id": record.get("project_id"),
            "iid": record.get("iid"),
            "id": record.get("id"),
            "title": title,
            "title_truncated": title_truncated,
            "description": description,
            "description_truncated": description_truncated,
            "state": record.get("state"),
            "draft": record.get("draft"),
            "author_id": author.get("id"),
            "author_username": author.get("username"),
            "merged_by_id": merged_by.get("id"),
            "merged_by_username": merged_by.get("username"),
            "source_branch": record.get("source_branch"),
            "target_branch": record.get("target_branch"),
            "created_at": record.get("created_at"),
            "updated_at": record.get("updated_at"),
            "merged_at": record.get("merged_at"),
            "closed_at": record.get("closed_at"),
            "sha": record.get("sha"),
            "merge_commit_sha": record.get("merge_commit_sha"),
            "squash_commit_sha": record.get("squash_commit_sha"),
            "squash": record.get("squash"),
            "merge_status": record.get("merge_status"),
            "changes_count": record.get("changes_count"),
            "user_notes_count": record.get("user_notes_count"),
            "milestone_id": milestone.get("id"),
            "assignee_ids": json.dumps([a.get("id") for a in assignees]),
            "reviewer_ids": json.dumps([r.get("id") for r in reviewers]),
            "labels": json.dumps(record.get("labels") or []),
        }
