"""Bitbucket Cloud branches stream (incremental by target.date, HttpSubStream of repositories)."""

import logging
from typing import Any, Iterable, Mapping, MutableMapping, Optional

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams.http import HttpSubStream

from source_bitbucket_cloud.streams.base import (
    BitbucketCloudStream,
    _make_unique_key,
    _normalize_start_date,
)
from source_bitbucket_cloud.streams.repositories import RepositoriesStream

logger = logging.getLogger("airbyte")


class BranchesStream(HttpSubStream, BitbucketCloudStream):
    """Branches for each repository.

    Incremental per-branch state: ``{ws/slug/branch: {head_sha, target_date}}``.
    A per-repo date cursor is unsafe here because force-pushes can rewrite a
    branch to a commit with an older author_date than the cursor, making the
    rewritten branch sort below the early-exit threshold and silently vanish
    from subsequent syncs. Hash-based comparison avoids that class of miss.

    Deleted branches are not re-emitted (no API signal). This is acceptable: bronze
    is a data lake, keeping historical branch rows is desirable.
    """

    name = "branches"
    cursor_field = "target_date"
    use_cache = True
    ignore_404 = True
    # State persists only at slice (per-repo) boundaries: all per-branch
    # entries for a repo land atomically after pagination completes.
    state_checkpoint_interval = None

    def __init__(
        self,
        parent: RepositoriesStream,
        start_date: Optional[str] = None,
        **kwargs: Any,
    ) -> None:
        """Parent slice records must carry: workspace, slug, mainbranch_name,
        updated_on. Parse failures here produce clear KeyError at parse_response
        rather than silent data issues downstream.
        """
        super().__init__(parent=parent, **kwargs)
        self._start_date = _normalize_start_date(start_date)
        self._stop_pagination: bool = False

    def _path(self, stream_slice: Optional[Mapping[str, Any]] = None) -> str:
        s = stream_slice or {}
        repo = s["parent"]
        return f"repositories/{repo['workspace']}/{repo['slug']}/refs/branches"

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
                    continue
                prefix = f"{workspace}/{slug}/"
                # Build the per-branch head map for this repo from state so
                # parse_response can skip emitting branches whose HEAD is
                # unchanged without relying on the (force-push-unsafe) date
                # cursor.
                branch_heads = {
                    k[len(prefix):]: (v or {}).get("head_sha", "") or ""
                    for k, v in state.items()
                    if k.startswith(prefix) and isinstance(v, dict)
                }
                self._stop_pagination = False
                slice_count += 1
                yield {
                    "parent": repo_record,
                    "branch_heads": branch_heads,
                    "has_prior_state": bool(branch_heads),
                }
        logger.info(f"branches: iterated {slice_count} repo slices")

    def request_params(
        self,
        stream_state: Optional[Mapping[str, Any]] = None,
        stream_slice: Optional[Mapping[str, Any]] = None,
        next_page_token: Optional[Mapping[str, Any]] = None,
    ) -> Mapping[str, Any]:
        if next_page_token:
            return {}
        params: dict[str, Any] = {
            "pagelen": str(self.page_size),
            "sort": "-target.date",
        }
        # No BBQL q-filter: Bitbucket docs only document `q=name~"..."` for
        # the refs endpoint; `target.date` appears to return 0 rows in
        # practice (see branches stream regression on prod). Incremental
        # filtering happens client-side via per-branch head_sha comparison
        # in parse_response. sort=-target.date is kept so the first run's
        # start_date cutoff can stop pagination once we cross below it.
        return params

    def next_page_token(self, response):
        if self._stop_pagination:
            self._stop_pagination = False
            return None
        return super().next_page_token(response)

    def parse_response(
        self,
        response,
        stream_slice: Optional[Mapping[str, Any]] = None,
        **kwargs: Any,
    ):
        s = stream_slice or {}
        repo = s["parent"]
        workspace = repo["workspace"]
        slug = repo["slug"]
        default_branch_name = repo.get("mainbranch_name", "")
        repo_updated_on = repo.get("updated_on", "")
        branch_heads: Mapping[str, str] = s.get("branch_heads") or {}
        has_prior_state: bool = bool(s.get("has_prior_state"))
        emitted = 0
        skipped_unchanged = 0

        for branch in self._iter_values(response):
            branch_name = branch.get("name", "")
            target = branch.get("target") or {}
            target_hash = target.get("hash", "") or ""
            target_date = target.get("date", "") or ""

            stored_hash = branch_heads.get(branch_name, "") or ""

            # HEAD-unchanged skip: if stored head_sha matches current target_hash,
            # the branch has no new commits (and no force-push). Skip emitting;
            # commits stream's own HEAD-unchanged guard also short-circuits.
            # Crucially, we do NOT stop pagination here — a later branch on a
            # following page may have been force-pushed to an older-dated commit
            # and must still be visited.
            if stored_hash and target_hash and stored_hash == target_hash:
                skipped_unchanged += 1
                continue

            # start_date cutoff only on true first run (no stored state for
            # any branch in this repo). With prior state, older pages may
            # still contain force-pushed branches that need emission.
            if (
                not has_prior_state
                and self._start_date
                and target_date
                and target_date[:10] < self._start_date
            ):
                self._stop_pagination = True
                logger.info(
                    f"branches: {workspace}/{slug} start_date cutoff at "
                    f"target_date={target_date} start_date={self._start_date}"
                )
                return
            emitted += 1

            record = {
                "unique_key": _make_unique_key(
                    self._tenant_id, self._source_id, workspace, slug, branch_name,
                ),
                "name": branch_name,
                "target_hash": target_hash,
                "target_date": target_date,
                "workspace": workspace,
                "repo_slug": slug,
                "mainbranch_name": default_branch_name,
                "default_branch_name": default_branch_name,
                "is_default": branch_name == default_branch_name,
                "updated_on": repo_updated_on,
            }
            yield self._envelope(record)

        logger.debug(
            f"branches: repo={workspace}/{slug} page_emitted={emitted} "
            f"skipped_unchanged={skipped_unchanged}"
        )

    def get_updated_state(
        self,
        current_stream_state: MutableMapping[str, Any],
        latest_record: Mapping[str, Any],
    ) -> MutableMapping[str, Any]:
        workspace = latest_record.get("workspace", "")
        slug = latest_record.get("repo_slug", "")
        branch_name = latest_record.get("name", "") or ""
        if not workspace or not slug or not branch_name:
            return current_stream_state
        partition_key = f"{workspace}/{slug}/{branch_name}"
        target_hash = latest_record.get("target_hash", "") or ""
        target_date = latest_record.get(self.cursor_field, "") or ""
        entry = dict(current_stream_state.get(partition_key, {}) or {})
        if target_hash:
            entry["head_sha"] = target_hash
        if target_date:
            entry[self.cursor_field] = target_date
        if entry:
            current_stream_state[partition_key] = entry
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
                "name": {"type": ["null", "string"]},
                "target_hash": {"type": ["null", "string"]},
                "target_date": {"type": ["null", "string"]},
                "workspace": {"type": "string"},
                "repo_slug": {"type": "string"},
                "mainbranch_name": {"type": ["null", "string"]},
                "default_branch_name": {"type": ["null", "string"]},
                "is_default": {"type": ["null", "boolean"]},
                "updated_on": {"type": ["null", "string"]},
            },
        }
