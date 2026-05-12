"""Bitbucket Cloud PR comments stream (incremental, per-PR, HttpSubStream of pull_requests)."""

import logging
from typing import Any, Iterable, Mapping, MutableMapping, Optional

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams.http import HttpSubStream

from source_bitbucket_cloud.streams.base import BitbucketCloudStream, _make_unique_key, _truncate


logger = logging.getLogger("airbyte")


class PRCommentsStream(HttpSubStream, BitbucketCloudStream):
    """Comments per PR. Per-PR incremental state keyed by ``pull_request_updated_on``.

    - ``HttpSubStream`` re-iterates the parent PRs stream in full_refresh to build
      slices. Each PR slice is then gated by our own state so unchanged PRs are
      skipped.
    - PRs with ``comment_count == 0`` are skipped entirely (no API call).
    """

    name = "pull_request_comments"
    cursor_field = "pull_request_updated_on"
    # Per-record get_updated_state marks the whole PR synced; mid-slice
    # checkpointing would therefore complete a PR after comment #1 and
    # drop remaining comments on crash. State persists only at slice
    # (per-PR) boundaries.
    state_checkpoint_interval = None
    ignore_404 = True

    def _path(self, stream_slice: Optional[Mapping[str, Any]] = None) -> str:
        s = stream_slice or {}
        pr = s["parent"]
        return (
            f"repositories/{pr['workspace']}/{pr['repo_slug']}/"
            f"pullrequests/{pr['id']}/comments"
        )

    def stream_slices(
        self,
        sync_mode: SyncMode,
        cursor_field: Optional[list] = None,
        stream_state: Optional[Mapping[str, Any]] = None,
    ) -> Iterable[Optional[Mapping[str, Any]]]:
        # Iterate parent via stream_slices + read_records directly (not via
        # HttpSubStream/read_only_records) — Stream.read() overrides incoming
        # stream_state with self.state, skipping PRs the child still needs
        # after a mid-stream crash. read_records honours the passed state.
        state = stream_state or {}
        total = 0
        skipped_unchanged = 0
        skipped_no_comments = 0

        for repo_slice in self.parent.stream_slices(
            sync_mode=SyncMode.full_refresh, cursor_field=None, stream_state={},
        ):
            for pr in self.parent.read_records(
                sync_mode=SyncMode.full_refresh,
                stream_slice=repo_slice,
                stream_state={},
            ):
                if not isinstance(pr, Mapping):
                    continue
                if not pr.get("comment_count"):
                    skipped_no_comments += 1
                    continue
                total += 1

                workspace = pr.get("workspace", "")
                slug = pr.get("repo_slug", "")
                pr_id = pr.get("id")
                pr_updated_on = pr.get("updated_on", "") or ""
                partition_key = f"{workspace}/{slug}/{pr_id}"

                synced_at = (state.get(partition_key, {}) or {}).get(self.cursor_field, "") or ""
                if pr_updated_on and synced_at and pr_updated_on <= synced_at:
                    skipped_unchanged += 1
                    continue

                yield {"parent": pr}

        logger.info(
            f"pull_request_comments: {total - skipped_unchanged} PRs to fetch "
            f"({skipped_unchanged} unchanged, {skipped_no_comments} zero-comment, "
            f"state_entries={len(state)})"
        )

    def parse_response(
        self,
        response,
        stream_slice: Optional[Mapping[str, Any]] = None,
        **kwargs: Any,
    ):
        s = stream_slice or {}
        pr = s["parent"]
        workspace = pr["workspace"]
        slug = pr["repo_slug"]
        pr_id = pr["id"]
        pr_updated_on = pr.get("updated_on", "")

        emitted = 0
        for comment in self._iter_values(response):
            comment_id = comment.get("id")
            if comment_id is None:
                continue
            emitted += 1
            user = comment.get("user") or {}
            content = comment.get("content") or {}
            inline = comment.get("inline")
            parent_comment = comment.get("parent")

            record = {
                "unique_key": _make_unique_key(
                    self._tenant_id, self._source_id,
                    workspace, slug, str(pr_id), str(comment_id),
                ),
                "comment_id": comment_id,
                "pr_id": pr_id,
                "body": _truncate(content.get("raw")),
                "created_on": comment.get("created_on"),
                "updated_on": comment.get("updated_on"),
                "author_display_name": user.get("display_name"),
                "author_uuid": user.get("uuid"),
                "is_inline": inline is not None,
                "inline_path": (inline or {}).get("path"),
                "inline_from": (inline or {}).get("from"),
                "inline_to": (inline or {}).get("to"),
                "parent_comment_id": parent_comment.get("id") if parent_comment else None,
                "is_deleted": comment.get("deleted", False),
                "pull_request_updated_on": pr_updated_on,
                "workspace": workspace,
                "repo_slug": slug,
            }
            yield self._envelope(record)

        logger.debug(
            f"pull_request_comments: {workspace}/{slug}/pr={pr_id} page emitted={emitted}"
        )

    def get_updated_state(
        self,
        current_stream_state: MutableMapping[str, Any],
        latest_record: Mapping[str, Any],
    ) -> MutableMapping[str, Any]:
        workspace = latest_record.get("workspace", "")
        slug = latest_record.get("repo_slug", "")
        pr_id = latest_record.get("pr_id")
        if not (workspace and slug) or pr_id is None:
            return current_stream_state
        partition_key = f"{workspace}/{slug}/{pr_id}"
        pr_updated_on = latest_record.get(self.cursor_field, "") or ""
        if pr_updated_on:
            current_stream_state[partition_key] = {self.cursor_field: pr_updated_on}
        return current_stream_state

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
                "comment_id": {"type": ["null", "integer"]},
                "pr_id": {"type": ["null", "integer"]},
                "body": {"type": ["null", "string"]},
                "created_on": {"type": ["null", "string"]},
                "updated_on": {"type": ["null", "string"]},
                "author_display_name": {"type": ["null", "string"]},
                "author_uuid": {"type": ["null", "string"]},
                "is_inline": {"type": ["null", "boolean"]},
                "inline_path": {"type": ["null", "string"]},
                "inline_from": {"type": ["null", "integer"]},
                "inline_to": {"type": ["null", "integer"]},
                "parent_comment_id": {"type": ["null", "integer"]},
                "is_deleted": {"type": ["null", "boolean"]},
                "pull_request_updated_on": {"type": ["null", "string"]},
                "workspace": {"type": "string"},
                "repo_slug": {"type": "string"},
            },
        }
