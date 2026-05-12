"""Bitbucket Cloud pull_requests stream (incremental, per-repo cursor)."""

import logging
from typing import Any, Iterable, Mapping, MutableMapping, Optional

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams.http import HttpSubStream

from source_bitbucket_cloud.streams.base import (
    BitbucketCloudStream,
    _make_unique_key,
    _normalize_start_date,
    _truncate,
)


logger = logging.getLogger("airbyte")


class PullRequestsStream(HttpSubStream, BitbucketCloudStream):
    """PRs for each repo; incremental by `updated_on`.

    Child streams (pr_comments, pr_commits) use this stream as their parent
    and re-iterate via HttpSubStream; the PRs HTTP responses are served from
    requests-cache (``use_cache=True``) on the re-iteration.
    """

    name = "pull_requests"
    cursor_field = "updated_on"
    use_cache = True
    # Descending API sort (sort=-updated_on): mid-slice checkpointing would
    # persist the NEWEST cursor after record #1 and a crash before slice
    # completion would cause the next run to skip all remaining (older)
    # records. State persists only at slice (per-repo) boundaries.
    state_checkpoint_interval = None

    # PRs API doesn't accept a generic ``pagelen`` for big pages — keep 50.
    page_size = 50

    def __init__(
        self,
        parent,  # RepositoriesStream
        start_date: Optional[str] = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(parent=parent, **kwargs)
        self._start_date = _normalize_start_date(start_date)
        self._stop_pagination: bool = False

    # ------------------------------------------------------------------
    # Path / params
    # ------------------------------------------------------------------

    def _path(self, stream_slice: Optional[Mapping[str, Any]] = None) -> str:
        repo = (stream_slice or {}).get("parent") or {}
        workspace = repo.get("workspace")
        slug = repo.get("slug")
        if not workspace or not slug:
            raise ValueError("pull_requests stream_slice requires parent.workspace and parent.slug")
        return f"repositories/{workspace}/{slug}/pullrequests"

    def request_params(
        self,
        stream_state: Optional[Mapping[str, Any]] = None,
        stream_slice: Optional[Mapping[str, Any]] = None,
        next_page_token: Optional[Mapping[str, Any]] = None,
    ) -> Mapping[str, Any]:
        if next_page_token:
            return {}
        return {
            "pagelen": str(self.page_size),
            "state": ["OPEN", "MERGED", "DECLINED", "SUPERSEDED"],
            "sort": "-updated_on",
        }

    def next_page_token(self, response):
        if self._stop_pagination:
            self._stop_pagination = False
            return None
        return super().next_page_token(response)

    # ------------------------------------------------------------------
    # Slices — attach cursor_value
    # ------------------------------------------------------------------

    def stream_slices(
        self,
        sync_mode: SyncMode,
        cursor_field: Optional[list] = None,
        stream_state: Optional[Mapping[str, Any]] = None,
    ) -> Iterable[Optional[Mapping[str, Any]]]:
        # Iterate parent via stream_slices + read_records directly (not via
        # HttpSubStream/read_only_records) — Stream.read() in CDK 7.x overrides
        # the incoming stream_state with self.state (parent's persistent cursor),
        # which skips slices that the child still needs to process after a
        # mid-stream crash. read_records() honours the passed stream_state.
        state = stream_state or {}
        slice_count = 0
        for repo_slice in self.parent.stream_slices(
            sync_mode=SyncMode.full_refresh, cursor_field=None, stream_state={},
        ):
            for repo_record in self.parent.read_records(
                sync_mode=SyncMode.full_refresh,
                stream_slice=repo_slice,
                stream_state={},
            ):
                if not isinstance(repo_record, Mapping):
                    continue
                workspace = repo_record.get("workspace")
                slug = repo_record.get("slug")
                if not workspace or not slug:
                    logger.warning(
                        f"pull_requests: skipping repo missing workspace/slug: {repo_record!r}"
                    )
                    continue
                partition_key = f"{workspace}/{slug}"
                cursor = (state.get(partition_key, {}) or {}).get(self.cursor_field, "") or ""
                # Reset per-slice so a prior early-exit doesn't leak into this repo.
                self._stop_pagination = False
                slice_count += 1
                logger.info(
                    f"pull_requests: slice={partition_key} cursor={cursor or '<none>'} "
                    f"start_date={self._start_date or '<none>'}"
                )
                yield {
                    "parent": repo_record,
                    "cursor_value": cursor,
                    "partition_key": partition_key,
                }
        logger.info(f"pull_requests: iterated {slice_count} repo slices")

    # ------------------------------------------------------------------
    # Parse
    # ------------------------------------------------------------------

    def parse_response(
        self,
        response,
        stream_slice: Optional[Mapping[str, Any]] = None,
        **kwargs: Any,
    ):
        s = stream_slice or {}
        repo = s.get("parent") or {}
        workspace = repo.get("workspace", "")
        slug = repo.get("slug", "")
        cursor_value = s.get("cursor_value", "") or ""

        emitted = 0
        for pr in self._iter_values(response):
            pr_id = pr.get("id")
            updated_on = pr.get("updated_on", "") or ""

            # Incremental early exit (PRs are sorted -updated_on).
            if cursor_value and updated_on and updated_on <= cursor_value:
                self._stop_pagination = True
                logger.info(
                    f"pull_requests: {workspace}/{slug} cursor early-exit "
                    f"at pr={pr_id} updated_on={updated_on} cursor={cursor_value} "
                    f"(emitted {emitted} this page)"
                )
                return
            if self._start_date and updated_on and updated_on[:10] < self._start_date:
                self._stop_pagination = True
                logger.info(
                    f"pull_requests: {workspace}/{slug} start_date cutoff "
                    f"at pr={pr_id} updated_on={updated_on} (emitted {emitted} this page)"
                )
                return
            emitted += 1

            author = pr.get("author") or {}
            src_branch = (pr.get("source") or {}).get("branch") or {}
            dst_branch = (pr.get("destination") or {}).get("branch") or {}
            merge_commit = pr.get("merge_commit") or {}

            participants = []
            for p in pr.get("participants") or []:
                user = p.get("user") or {}
                participants.append({
                    "display_name": user.get("display_name"),
                    "uuid": user.get("uuid"),
                    "nickname": user.get("nickname"),
                    "role": p.get("role"),
                    "approved": p.get("approved", False),
                    "state": p.get("state"),
                })

            record = {
                "unique_key": _make_unique_key(
                    self._tenant_id, self._source_id, workspace, slug, str(pr_id),
                ),
                "id": pr_id,
                "title": pr.get("title"),
                "description": _truncate(pr.get("description")),
                "state": pr.get("state"),
                "created_on": pr.get("created_on"),
                "updated_on": updated_on,
                "author_display_name": author.get("display_name"),
                "author_uuid": author.get("uuid"),
                "source_branch": src_branch.get("name"),
                "destination_branch": dst_branch.get("name"),
                "merge_commit_hash": merge_commit.get("hash"),
                # comment_count retained: read by pr_comments.stream_slices
                # to skip zero-comment PRs without an API call.
                "comment_count": pr.get("comment_count", 0),
                "participants": participants,
                "workspace": workspace,
                "repo_slug": slug,
            }
            yield self._envelope(record)

        logger.debug(f"pull_requests: {workspace}/{slug} page emitted={emitted}")

    # ------------------------------------------------------------------
    # State — per-repo cursor
    # ------------------------------------------------------------------

    def get_updated_state(
        self,
        current_stream_state: MutableMapping[str, Any],
        latest_record: Mapping[str, Any],
    ) -> MutableMapping[str, Any]:
        partition_key = f"{latest_record.get('workspace', '')}/{latest_record.get('repo_slug', '')}"
        record_cursor = latest_record.get(self.cursor_field, "") or ""
        entry = dict(current_stream_state.get(partition_key, {}) or {})
        prev = entry.get(self.cursor_field, "") or ""
        if record_cursor and record_cursor > prev:
            entry[self.cursor_field] = record_cursor
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
                "id": {"type": ["null", "integer"]},
                "title": {"type": ["null", "string"]},
                "description": {"type": ["null", "string"]},
                "state": {"type": ["null", "string"]},
                "created_on": {"type": ["null", "string"]},
                "updated_on": {"type": ["null", "string"]},
                "author_display_name": {"type": ["null", "string"]},
                "author_uuid": {"type": ["null", "string"]},
                "source_branch": {"type": ["null", "string"]},
                "destination_branch": {"type": ["null", "string"]},
                "merge_commit_hash": {"type": ["null", "string"]},
                "comment_count": {"type": ["null", "integer"]},
                "participants": {"type": ["null", "array"]},
                "workspace": {"type": "string"},
                "repo_slug": {"type": "string"},
            },
        }
