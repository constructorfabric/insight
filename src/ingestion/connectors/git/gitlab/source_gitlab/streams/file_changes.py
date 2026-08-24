from __future__ import annotations

from collections.abc import Iterable, Mapping, MutableMapping
from typing import Any

from airbyte_cdk.models import AirbyteMessage, SyncMode
from airbyte_cdk.sources.streams import IncrementalMixin

from source_gitlab.streams import concurrency
from source_gitlab.streams.base import GitlabStream, GitlabSubstream, parse_diff_counts
from source_gitlab.streams.concurrency import RequestGate
from source_gitlab.streams.windowing import CommittedDateWindowing

_RefKey = tuple[Any, str | None]


class _HeadFrontier:
    """Advance a walked ref's stored head only after all its diff tasks are emitted.

    Keyed by (project_id, branch), where branch is None for the default one.
    A branch whose diffs partly failed keeps its old head, so the next sync
    re-walks the range instead of stepping over the gap.
    """

    def __init__(self, stream: CommitFileChangesStream) -> None:
        self._stream = stream
        self._pending: dict[_RefKey, dict[str, Any]] = {}

    def open(self, key: _RefKey, head: str) -> None:
        self._pending[key] = {
            "head": head,
            "remaining": 0,
            "done": False,
            "advanced": False,
        }

    def add_one(self, key: _RefKey) -> None:
        self._pending[key]["remaining"] += 1

    def finish_enum(self, key: _RefKey) -> None:
        self._pending[key]["done"] = True
        self._maybe_advance(key)

    def complete_one(self, key: _RefKey) -> None:
        entry = self._pending.get(key)
        if entry is None:
            return
        entry["remaining"] -= 1
        self._maybe_advance(key)

    def _maybe_advance(self, key: _RefKey) -> None:
        entry = self._pending[key]
        if not (entry["done"] and entry["remaining"] == 0 and not entry["advanced"]):
            return
        project_id, branch = key
        pstate = self._stream._project_state(project_id)
        if branch is None:
            pstate["default_head"] = entry["head"]
        else:
            pstate.setdefault("branches", {})[branch] = entry["head"]
        entry["advanced"] = True


class CommitFileChangesStream(GitlabSubstream, IncrementalMixin):
    name = "commit_file_changes"
    cursor_field = "commit_sha"
    state_checkpoint_interval = 1000
    skippable_statuses: frozenset[int] = frozenset()

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
        frontier = _HeadFrontier(self)
        for task, records in concurrency.imap_bounded(
            self._gate, self._diff_tasks(frontier), self._fetch_diff
        ):
            yield from records
            frontier.complete_one(task["ref_key"])

    def _diff_tasks(self, frontier: _HeadFrontier) -> Iterable[Mapping[str, Any]]:
        for project in self._iter_unique_projects():
            project_id = project.get("id")
            default = project.get("default_branch")
            if project_id is None or not default:
                continue
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
                continue
            pstate = self._state.get("projects", {}).get(str(project_id), {})

            stored = pstate.get("default_head")
            if not stored:
                yield from self._walk_ref(frontier, project_id, None, default, default_head)
            elif stored != default_head:
                yield from self._walk_ref(
                    frontier, project_id, None, f"{stored}..{default_head}", default_head
                )

            # Mirrors the commits stream: the default ref is walked whole and
            # every other branch only as `default_head..head`, so a commit is
            # diffed under exactly one ref per sync. A branch head equal to the
            # default's adds nothing.
            stored_branches = pstate.get("branches") or {}
            for branch in branch_records:
                name = branch.get("name")
                if name == default:
                    continue
                head = branch.get("commit_sha")
                if not head or head == default_head:
                    continue
                if stored_branches.get(name) == head:
                    continue
                yield from self._walk_ref(
                    frontier,
                    project_id,
                    name,
                    f"{default_head}..{head}",
                    head,
                    # A head that moved on since the branch listing leaves the
                    # range unresolvable; skipping costs this branch one sync,
                    # while raising would cost the whole run.
                    skippable=frozenset({404}),
                )

            self._prune_branches(project_id, {b.get("name") for b in branch_records})

    def _walk_ref(
        self,
        frontier: _HeadFrontier,
        project_id: Any,
        branch: str | None,
        ref: str,
        head: str,
        *,
        skippable: frozenset[int] = frozenset(),
    ) -> Iterable[Mapping[str, Any]]:
        key: _RefKey = (project_id, branch)
        frontier.open(key, head)
        for sha in self._iter_shas(
            {"project_id": project_id, "ref": ref}, skippable=skippable
        ):
            frontier.add_one(key)
            yield {"project_id": project_id, "sha": sha, "ref_key": key}
        frontier.finish_enum(key)

    def _prune_branches(self, project_id: Any, current: set[Any]) -> None:
        stored = self._state.get("projects", {}).get(str(project_id), {}).get("branches")
        if not stored:
            return
        self._project_state(project_id)["branches"] = {
            name: head for name, head in stored.items() if name in current
        }

    def _iter_shas(
        self, enum_slice: Mapping[str, Any], *, skippable: frozenset[int] = frozenset()
    ) -> Iterable[str]:
        base = {**enum_slice, "since": self._start_date}
        for commit in concurrency.walk_window(
            strategy=self._strategy,
            base_slice=base,
            url_base=self.url_base,
            path_fn=self._commit_path,
            params_fn=self._commit_params,
            envelope_fn=self._commit_min,
            headers=self._headers(),
            gate=self._gate,
            skippable=skippable,
        ):
            if commit.get("id") and (commit.get("parent_count") or 0) <= 1:
                yield str(commit["id"])

    def _fetch_diff(
        self, task: Mapping[str, Any]
    ) -> tuple[Mapping[str, Any], list[Mapping[str, Any]]]:
        records = list(
            concurrency.paginate(
                self._gate,
                url_base=self.url_base,
                path=self._path(stream_slice=task),
                params=self._initial_params(task),
                envelope_fn=lambda raw: self._envelope(raw, task),
                headers=self._headers(),
                skippable=self.skippable_statuses,
            )
        )
        return task, records

    def _headers(self) -> Mapping[str, str]:
        return {"PRIVATE-TOKEN": self._token}

    def _commit_path(self, stream_slice: Mapping[str, Any]) -> str:
        return f"projects/{stream_slice['project_id']}/repository/commits"

    def _commit_params(self, stream_slice: Mapping[str, Any]) -> Mapping[str, Any]:
        params: dict[str, Any] = {
            "ref_name": stream_slice["ref"],
            "per_page": self.page_size,
        }
        if stream_slice.get("since"):
            params["since"] = stream_slice["since"]
        if stream_slice.get("until"):
            params["until"] = stream_slice["until"]
        return params

    def _commit_min(
        self, raw: Mapping[str, Any], stream_slice: Mapping[str, Any]
    ) -> Mapping[str, Any]:
        return {"id": raw.get("id"), "parent_count": len(raw.get("parent_ids") or [])}

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
