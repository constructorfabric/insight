"""Bitbucket Cloud PR commits stream (incremental, per-PR, HttpSubStream of pull_requests)."""

import logging
from typing import Any, Iterable, Mapping, MutableMapping, Optional

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams.http import HttpSubStream

from source_bitbucket_cloud.streams.base import BitbucketCloudStream, _make_unique_key


logger = logging.getLogger("airbyte")


class PRCommitsStream(HttpSubStream, BitbucketCloudStream):
    """Commits per PR. Per-PR incremental state keyed by ``pull_request_updated_on``.

    ``HttpSubStream`` re-iterates parent PRs to build slices; our own state
    skips PRs whose ``updated_on`` hasn't moved since last sync.
    """

    name = "pull_request_commits"
    cursor_field = "pull_request_updated_on"
    # Per-record get_updated_state marks the whole PR synced; mid-slice
    # checkpointing would therefore complete a PR after commit #1 and
    # drop remaining commits on crash. State persists only at slice
    # (per-PR) boundaries.
    state_checkpoint_interval = None
    ignore_404 = True

    def _path(self, stream_slice: Optional[Mapping[str, Any]] = None) -> str:
        s = stream_slice or {}
        pr = s["parent"]
        return (
            f"repositories/{pr['workspace']}/{pr['repo_slug']}/"
            f"pullrequests/{pr['id']}/commits"
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
            f"pull_request_commits: {total - skipped_unchanged} PRs to fetch "
            f"({skipped_unchanged} unchanged, state_entries={len(state)})"
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
        for commit in self._iter_values(response):
            commit_hash = commit.get("hash", "") or ""
            if not commit_hash:
                continue
            emitted += 1
            author_user = (commit.get("author") or {}).get("user") or {}

            # message/date/author_name/author_email intentionally omitted:
            # the `commits` stream carries the full commit record, joined
            # downstream by hash. Only hash + PR linkage is needed here.
            record = {
                "unique_key": _make_unique_key(
                    self._tenant_id, self._source_id,
                    workspace, slug, str(pr_id), commit_hash,
                ),
                "pr_id": pr_id,
                "hash": commit_hash,
                "author_uuid": author_user.get("uuid"),
                "pull_request_updated_on": pr_updated_on,
                "workspace": workspace,
                "repo_slug": slug,
            }
            yield self._envelope(record)

        logger.debug(
            f"pull_request_commits: {workspace}/{slug}/pr={pr_id} page emitted={emitted}"
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
                "pr_id": {"type": ["null", "integer"]},
                "hash": {"type": ["null", "string"]},
                "author_uuid": {"type": ["null", "string"]},
                "pull_request_updated_on": {"type": ["null", "string"]},
                "workspace": {"type": "string"},
                "repo_slug": {"type": "string"},
            },
        }
