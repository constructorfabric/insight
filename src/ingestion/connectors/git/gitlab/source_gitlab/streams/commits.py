from __future__ import annotations

from collections.abc import Generator, Iterable, Iterator, Mapping, MutableMapping
from typing import Any

from airbyte_cdk.models import AirbyteMessage, SyncMode
from airbyte_cdk.sources.streams import IncrementalMixin

from source_gitlab.streams import concurrency
from source_gitlab.streams.base import (
    MAX_BODY_CHARS,
    MAX_TITLE_CHARS,
    GitlabStream,
    GitlabSubstream,
    trim_text,
)
from source_gitlab.streams.concurrency import RequestGate
from source_gitlab.streams.windowing import CommittedDateWindowing


class CommitsStream(GitlabSubstream, IncrementalMixin):
    name = "commits"
    cursor_field = "committed_date"
    state_checkpoint_interval = 1000

    def __init__(
        self,
        *,
        parent: GitlabStream,
        branches: GitlabStream,
        gate: RequestGate,
        start_date: str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(parent=parent, **kwargs)
        self._branches = branches
        self._gate = gate
        self._start_date = start_date
        self._strategy = CommittedDateWindowing()
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
        yield {}

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any] | AirbyteMessage]:
        for task, item in concurrency.imap_stream(
            self._gate, self._project_tasks(), self._fetch_project
        ):
            if isinstance(item, concurrency.Done):
                self._apply(task, item.value)
            else:
                yield item

    def _project_tasks(self) -> Iterable[Mapping[str, Any]]:
        for project in self._iter_unique_projects():
            project_id = project.get("id")
            default = project.get("default_branch")
            if project_id is None or not default:
                continue
            yield {
                "project": project,
                "project_id": project_id,
                "default": default,
                "snapshot": self._snapshot(project_id),
            }

    def _snapshot(self, project_id: Any) -> Mapping[str, Any]:
        stored = self._state.get("projects", {}).get(str(project_id), {})
        return {
            "default_head": stored.get("default_head"),
            "branches": dict(stored.get("branches") or {}),
        }

    def _fetch_project(
        self, task: Mapping[str, Any]
    ) -> Generator[Mapping[str, Any], None, Mapping[str, Any]]:
        project = task["project"]
        project_id = task["project_id"]
        default = task["default"]
        snapshot = task["snapshot"]
        branch_records = [
            b
            for b in self._branches.read_records(
                sync_mode=SyncMode.full_refresh, stream_slice={"parent": project}
            )
            if isinstance(b, Mapping)
        ]
        default_head = next(
            (b.get("commit_sha") for b in branch_records if b.get("name") == default),
            None,
        )
        if not default_head:
            return {"current_branches": None, "advances": []}
        advances: list[tuple[Any, ...]] = []
        stored_default = snapshot["default_head"]
        if not stored_default:
            yield from self._walk_ref(project_id, default)
            advances.append(("default", default_head))
        elif stored_default != default_head:
            yield from self._walk_ref(
                project_id, f"{stored_default}..{default_head}", skip_404=False
            )
            advances.append(("default", default_head))
        stored_branches = snapshot["branches"]
        for branch in branch_records:
            name = branch.get("name")
            if name == default:
                continue
            head = branch.get("commit_sha")
            if not head or head == default_head:
                continue
            if stored_branches.get(name) == head:
                continue
            yield from self._walk_ref(project_id, f"{default_head}..{head}")
            advances.append(("branch", name, head))
        return {
            "current_branches": {b.get("name") for b in branch_records},
            "advances": advances,
        }

    def _walk_ref(
        self, project_id: Any, ref: str, *, skip_404: bool = True
    ) -> Iterator[Mapping[str, Any]]:
        base: dict[str, Any] = {
            "project_id": project_id,
            "ref": ref,
            "since": self._start_date,
        }
        if not skip_404:
            base["skip_404"] = False
        yield from concurrency.walk_window(
            strategy=self._strategy,
            base_slice=base,
            url_base=self.url_base,
            path_fn=lambda applied: self._path(stream_slice=applied),
            params_fn=self._initial_params,
            envelope_fn=self._envelope,
            headers={"PRIVATE-TOKEN": self._token},
            gate=self._gate,
            skippable=self.skippable_statuses,
        )

    def _apply(self, task: Mapping[str, Any], outcome: Mapping[str, Any]) -> None:
        if outcome.get("current_branches") is None:
            return
        pstate = self._project_state(task["project_id"])
        stored_branches = pstate.get("branches")
        if stored_branches:
            current = outcome["current_branches"]
            pstate["branches"] = {
                k: v for k, v in stored_branches.items() if k in current
            }
        for advance in outcome["advances"]:
            if advance[0] == "default":
                pstate["default_head"] = advance[1]
            else:
                pstate.setdefault("branches", {})[advance[1]] = advance[2]

    def _path(self, *, stream_slice: Mapping[str, Any] | None) -> str:
        project_id = (stream_slice or {})["project_id"]
        return f"projects/{project_id}/repository/commits"

    def _initial_params(
        self, stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        s = stream_slice or {}
        params: dict[str, Any] = {
            "ref_name": s["ref"],
            "with_stats": "true",
            "per_page": self.page_size,
        }
        if s.get("since"):
            params["since"] = s["since"]
        if s.get("until"):
            params["until"] = s["until"]
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
