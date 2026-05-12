"""Bitbucket Cloud file_changes stream — per-commit diffstat.

Parent is ``commits``. file_changes consumes the records that the commits
stream emits (so the cross-branch bloom-filter dedup, HEAD-unchanged skip,
and force-push reset all happen once inside the commits stream and
file_changes inherits them for free).

file_changes has its own ``committed_date`` cursor + ``head_sha`` per branch.
When invoking the commits parent it translates its state into commits-state
shape so the parent's HEAD-unchanged skip and cursor early-exit fire
against *file_changes'* progress, not the commits stream's own progress.

This means: if commits stream already finished a branch in this run but
file_changes died mid-way through the same branch, the next run re-iterates
exactly the commits file_changes missed (not more, not less), because the
translated state carries file_changes' cursor + head_sha into the commits
re-invocation.
"""

import logging
from typing import Any, Dict, Iterable, List, Mapping, MutableMapping, Optional

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams.http import HttpSubStream

from source_bitbucket_cloud.streams.base import BitbucketCloudStream, _make_unique_key


logger = logging.getLogger("airbyte")


class FileChangesStream(HttpSubStream, BitbucketCloudStream):

    name = "file_changes"
    cursor_field = "committed_date"
    # cursor advances on the first diffstat row of each commit; mid-slice
    # checkpointing would therefore mark a commit done after file #1
    # and drop remaining files on crash. State persists only at slice
    # (per-commit) boundaries.
    state_checkpoint_interval = None
    ignore_404 = True

    def __init__(
        self,
        parent,  # CommitsStream
        start_date: Optional[str] = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(parent=parent, **kwargs)
        self._start_date = start_date
        self._current_partition_key: str = ""
        self._current_head_sha: str = ""

    # ------------------------------------------------------------------
    # Path — diffstat endpoint
    # ------------------------------------------------------------------

    def _path(self, stream_slice: Optional[Mapping[str, Any]] = None) -> str:
        s = stream_slice or {}
        workspace = s.get("workspace")
        slug = s.get("slug")
        sha = s.get("sha")
        if not workspace or not slug or not sha:
            raise ValueError("file_changes stream_slice requires 'workspace', 'slug', 'sha'")
        return f"repositories/{workspace}/{slug}/diffstat/{sha}"

    # ------------------------------------------------------------------
    # Slices — consume commits parent with translated state
    # ------------------------------------------------------------------

    def stream_slices(
        self,
        sync_mode: SyncMode,
        cursor_field: Optional[List[str]] = None,
        stream_state: Optional[Mapping[str, Any]] = None,
    ) -> Iterable[Optional[Mapping[str, Any]]]:
        # sync_mode/cursor_field ignored: file_changes is incremental-only and
        # always drives the commits parent with translated (committed_date) state.
        del sync_mode, cursor_field
        state = stream_state or {}
        translated = self._translate_state(state)
        logger.info(
            f"file_changes: stream_slices start state_entries={len(state)} "
            f"translated_entries={len(translated)} start_date={self._start_date or '<none>'}"
        )

        total = 0
        skipped_merge = 0

        # Drive the commits parent with *our* translated state so the parent
        # applies its HEAD-unchanged skip, force-push reset, and cursor
        # early-exit relative to file_changes' progress.
        for parent_slice in self.parent.stream_slices(
            sync_mode=SyncMode.incremental,
            cursor_field=["date"],
            stream_state=translated,
        ):
            branch = parent_slice["parent"]
            workspace = branch["workspace"]
            slug = branch["repo_slug"]
            branch_name = branch["name"]
            partition_key = f"{workspace}/{slug}/{branch_name}"
            current_head = branch.get("target_hash", "") or ""

            for commit_record in self.parent.read_records(
                sync_mode=SyncMode.incremental,
                cursor_field=["date"],
                stream_slice=parent_slice,
                stream_state=translated,
            ):
                parents = commit_record.get("parent_hashes") or []
                if len(parents) > 1:
                    skipped_merge += 1
                    continue

                sha = commit_record.get("hash", "") or ""
                committed_date = commit_record.get("date", "") or ""
                if not sha:
                    continue

                total += 1
                yield {
                    "workspace": workspace,
                    "slug": slug,
                    "branch": branch_name,
                    "sha": sha,
                    "committed_date": committed_date,
                    "partition_key": partition_key,
                    "head_sha": current_head,
                }

        logger.info(
            f"file_changes: {total} commits to diffstat ({skipped_merge} merge skipped)"
        )

    def _translate_state(self, state: Mapping[str, Any]) -> Dict[str, Dict[str, str]]:
        """Map file_changes state → commits-state shape.

        file_changes: {pk: {committed_date, head_sha}}
        commits:      {pk: {date, head_sha}}
        """
        translated: Dict[str, Dict[str, str]] = {}
        for pk, entry in (state or {}).items():
            if not isinstance(entry, dict):
                continue
            translated[pk] = {
                "date": entry.get(self.cursor_field, "") or "",
                "head_sha": entry.get("head_sha", "") or "",
            }
        return translated

    # ------------------------------------------------------------------
    # Parse diffstat response
    # ------------------------------------------------------------------

    def parse_response(
        self,
        response,
        stream_slice: Optional[Mapping[str, Any]] = None,
        **kwargs: Any,
    ):
        s = stream_slice or {}
        self._current_partition_key = s.get("partition_key", "")
        self._current_head_sha = s.get("head_sha", "")
        workspace = s.get("workspace", "")
        slug = s.get("slug", "")
        sha = s.get("sha", "")
        committed_date = s.get("committed_date", "")
        if not workspace or not slug or not sha:
            return

        emitted = 0
        for entry in self._iter_values(response):
            new_file = entry.get("new") or {}
            old_file = entry.get("old") or {}
            filename = new_file.get("path") or old_file.get("path") or ""
            if not filename:
                continue
            status = entry.get("status", "") or ""
            previous_filename = old_file.get("path") if status == "renamed" else None

            record = {
                "unique_key": _make_unique_key(
                    self._tenant_id, self._source_id, workspace, slug, sha, filename,
                ),
                "source_type": "commit",
                "sha": sha,
                "filename": filename,
                "status": status,
                "additions": entry.get("lines_added"),
                "deletions": entry.get("lines_removed"),
                "previous_filename": previous_filename,
                "committed_date": committed_date,
                "workspace": workspace,
                "repo_slug": slug,
            }
            emitted += 1
            yield self._envelope(record)

        logger.debug(
            f"file_changes: {workspace}/{slug}/{sha[:8]} diffstat emitted={emitted}"
        )

    # ------------------------------------------------------------------
    # State
    # ------------------------------------------------------------------

    def get_updated_state(
        self,
        current_stream_state: MutableMapping[str, Any],
        latest_record: Mapping[str, Any],
    ) -> MutableMapping[str, Any]:
        partition_key = self._current_partition_key
        if not partition_key:
            return current_stream_state
        record_date = latest_record.get(self.cursor_field, "") or ""
        entry = dict(current_stream_state.get(partition_key, {}) or {})
        prev_date = entry.get(self.cursor_field, "") or ""
        if record_date and record_date > prev_date:
            entry[self.cursor_field] = record_date
        if self._current_head_sha:
            entry["head_sha"] = self._current_head_sha
        if entry:
            current_stream_state[partition_key] = entry
        return current_stream_state

    # ------------------------------------------------------------------
    # Schema
    # ------------------------------------------------------------------

    def get_json_schema(self) -> Mapping[str, Any]:
        return {
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "tenant_id": {"type": "string"},
                "source_id": {"type": "string"},
                "unique_key": {"type": "string"},
                "data_source": {"type": "string"},
                "collected_at": {"type": "string"},
                "source_type": {"type": "string"},
                "sha": {"type": ["null", "string"]},
                "filename": {"type": ["null", "string"]},
                "status": {"type": ["null", "string"]},
                "additions": {"type": ["null", "integer"]},
                "deletions": {"type": ["null", "integer"]},
                "previous_filename": {"type": ["null", "string"]},
                "committed_date": {"type": ["null", "string"]},
                "workspace": {"type": "string"},
                "repo_slug": {"type": "string"},
            },
        }
