from __future__ import annotations

from collections.abc import Iterable, Mapping, MutableMapping
from typing import Any

from airbyte_cdk.models import AirbyteMessage, SyncMode
from airbyte_cdk.sources.streams import IncrementalMixin

from source_gitlab.streams.base import (
    MAX_BODY_CHARS,
    MAX_TITLE_CHARS,
    GitlabStream,
    GitlabSubstream,
    trim_text,
)
from source_gitlab.streams.windowing import CommittedDateWindowing


class CommitsStream(CommittedDateWindowing, GitlabSubstream, IncrementalMixin):
    name = "commits"
    cursor_field = "committed_date"

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
        self._start_date = start_date
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
                stored = self._state.get("projects", {}).get(str(project_id), {})
                if stored.get("branches"):
                    current = {b.get("name") for b in branch_records}
                    stored["branches"] = {
                        k: v for k, v in stored["branches"].items() if k in current
                    }
                stored_default = stored.get("default_head")
                if not stored_default:
                    yield {
                        "project_id": project_id,
                        "ref": default,
                        "advance": ("default", default_head),
                    }
                elif stored_default != default_head:
                    yield {
                        "project_id": project_id,
                        "ref": f"{stored_default}..{default_head}",
                        "advance": ("default", default_head),
                        "skip_404": False,
                    }
                stored_branches = stored.get("branches", {})
                for branch in branch_records:
                    name = branch.get("name")
                    if name == default:
                        continue
                    head = branch.get("commit_sha")
                    if not head or head == default_head:
                        continue
                    if stored_branches.get(name) == head:
                        continue
                    yield {
                        "project_id": project_id,
                        "ref": f"{default_head}..{head}",
                        "advance": ("branch", name, head),
                    }

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
        advance = (stream_slice or {}).get("advance")
        project_id = (stream_slice or {}).get("project_id")
        if advance and project_id is not None:
            pstate = self._project_state(project_id)
            if advance[0] == "default":
                pstate["default_head"] = advance[1]
            else:
                pstate.setdefault("branches", {})[advance[1]] = advance[2]

    def _path(self, *, stream_slice: Mapping[str, Any] | None) -> str:
        project_id = (stream_slice or {})["project_id"]
        return f"projects/{project_id}/repository/commits"

    def _window_initial(self, stream_slice: Mapping[str, Any] | None) -> dict[str, Any]:
        return {"since": self._start_date, "until": None}

    def _initial_params(
        self, stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        params: dict[str, Any] = {
            "ref_name": (stream_slice or {})["ref"],
            "with_stats": "true",
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
        project_id = (stream_slice or {})["project_id"]
        return [str(project_id), str(record["id"])]

    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        message, message_truncated = trim_text(record.get("message"), MAX_BODY_CHARS)
        title, title_truncated = trim_text(record.get("title"), MAX_TITLE_CHARS)
        stats = record.get("stats") or {}
        parents = record.get("parent_ids") or []
        return {
            "project_id": (stream_slice or {})["project_id"],
            "id": record.get("id"),
            "short_id": record.get("short_id"),
            "title": title,
            "title_truncated": title_truncated,
            "message": message,
            "message_truncated": message_truncated,
            "author_name": record.get("author_name"),
            "author_email": record.get("author_email"),
            "authored_date": record.get("authored_date"),
            "committer_name": record.get("committer_name"),
            "committer_email": record.get("committer_email"),
            "committed_date": record.get("committed_date"),
            "parent_count": len(parents),
            "stats_additions": stats.get("additions"),
            "stats_deletions": stats.get("deletions"),
            "stats_total": stats.get("total"),
        }
