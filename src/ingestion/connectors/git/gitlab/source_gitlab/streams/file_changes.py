from __future__ import annotations

from collections.abc import Iterable, Mapping, MutableMapping
from functools import cache
from typing import Any

from airbyte_cdk.models import AirbyteMessage, SyncMode
from airbyte_cdk.sources.streams import IncrementalMixin

from source_gitlab.streams.base import (
    GitlabStream,
    GitlabSubstream,
    parse_diff_counts,
)
from source_gitlab.streams.windowing import CommittedDateWindowing


class _DefaultCommitsEnumerator(CommittedDateWindowing, GitlabStream):
    name = "_default_commits_internal"

    def __init__(self, *, start_date: str | None = None, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self._start_date = start_date

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
        return f"projects/{(stream_slice or {})['project_id']}/repository/commits"

    def _window_initial(self, stream_slice: Mapping[str, Any] | None) -> dict[str, Any]:
        return {"since": self._start_date, "until": None}

    def _initial_params(
        self, stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        params: dict[str, Any] = {
            "ref_name": (stream_slice or {})["ref"],
            "per_page": self.page_size,
        }
        since = (stream_slice or {}).get("since")
        if since:
            params["since"] = since
        until = (stream_slice or {}).get("until")
        if until:
            params["until"] = until
        return params

    def _record_key(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> list[str]:
        return [str((stream_slice or {})["project_id"]), str(record["id"])]

    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        return {
            "id": record.get("id"),
            "parent_count": len(record.get("parent_ids") or []),
        }


class CommitFileChangesStream(GitlabSubstream, IncrementalMixin):
    name = "commit_file_changes"
    cursor_field = "commit_sha"
    skippable_statuses: frozenset[int] = frozenset()

    def __init__(
        self,
        *,
        parent: GitlabStream,
        branches: GitlabStream,
        start_date: str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(parent=parent, **kwargs)
        self._branches = branches
        self._enum = _DefaultCommitsEnumerator(
            base_url=self._base_url,
            token=self._token,
            tenant_id=self._tenant_id,
            source_id=self._source_id,
            start_date=start_date,
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
                default = project.get("default_branch")
                if project_id is None or not default:
                    continue
                branch_records = [
                    b
                    for b in self._branches.read_records(
                        sync_mode=SyncMode.full_refresh,
                        stream_slice={"parent": project},
                    )
                    if isinstance(b, Mapping)
                ]
                default_head = next(
                    (b.get("commit_sha") for b in branch_records if b.get("name") == default),
                    None,
                )
                if not default_head:
                    continue
                stored_default = (
                    self._state.get("projects", {}).get(str(project_id), {}).get("default_head")
                )
                if not stored_default:
                    enum_slice: dict[str, Any] = {"project_id": project_id, "ref": default}
                elif stored_default != default_head:
                    enum_slice = {
                        "project_id": project_id,
                        "ref": f"{stored_default}..{default_head}",
                    }
                else:
                    continue
                # One-item lookahead so only the final SHA carries the state
                # advance, without materialising the whole SHA list (bounded memory).
                pending: str | None = None
                for commit in self._enum.read_records(
                    sync_mode=SyncMode.full_refresh, stream_slice=enum_slice
                ):
                    if (
                        not isinstance(commit, Mapping)
                        or not commit.get("id")
                        or (commit.get("parent_count") or 0) > 1
                    ):
                        continue
                    if pending is not None:
                        yield {"project_id": project_id, "sha": pending}
                    pending = commit["id"]
                if pending is None:
                    self._project_state(project_id)["default_head"] = default_head
                    continue
                yield {"project_id": project_id, "sha": pending, "advance": default_head}

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
            self._project_state(project_id)["default_head"] = advance

    def _path(self, *, stream_slice: Mapping[str, Any] | None) -> str:
        s = stream_slice or {}
        return f"projects/{s['project_id']}/repository/commits/{s['sha']}/diff"

    def _initial_params(
        self, stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        return {"per_page": self.page_size}

    def _record_key(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> list[str]:
        s = stream_slice or {}
        path = record.get("new_path") or record.get("old_path") or ""
        return [str(s["project_id"]), str(s["sha"]), str(path)]

    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        s = stream_slice or {}
        added, removed, truncated = parse_diff_counts(record)
        return {
            "project_id": s["project_id"],
            "commit_sha": s["sha"],
            "old_path": record.get("old_path"),
            "new_path": record.get("new_path"),
            "new_file": record.get("new_file"),
            "deleted_file": record.get("deleted_file"),
            "renamed_file": record.get("renamed_file"),
            "lines_added": added,
            "lines_removed": removed,
            "diff_truncated": truncated,
        }
