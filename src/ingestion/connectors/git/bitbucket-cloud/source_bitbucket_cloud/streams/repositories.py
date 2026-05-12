"""Bitbucket Cloud repositories stream (incremental by updated_on)."""

import logging
from typing import Any, Iterable, Mapping, MutableMapping, Optional

from airbyte_cdk.models import SyncMode

from source_bitbucket_cloud.streams.base import (
    BitbucketCloudStream,
    _make_unique_key,
    _normalize_start_date,
)

logger = logging.getLogger("airbyte")


class RepositoriesStream(BitbucketCloudStream):
    """All repositories for each configured workspace.

    Incremental per-workspace cursor on ``updated_on``. Having a cursor_field
    skips the CDK's auto-assigned ResumableFullRefreshCursor so child streams
    (branches, pull_requests) can re-iterate this stream as a parent via
    HttpSubStream without hitting a "cursor already complete" state.

    When child streams call ``read_only_records(child_state)`` the child's
    state shape doesn't match our per-workspace shape, so the ``q`` filter
    silently falls back to unfiltered — children get the full repo list,
    which is what they need to iterate correctly.
    """

    name = "repositories"
    cursor_field = "updated_on"
    use_cache = True
    # Descending API sort (sort=-updated_on): mid-slice checkpointing would
    # persist the NEWEST cursor after record #1 and a crash before slice
    # completion would cause the next run to skip all remaining (older)
    # records. State persists only at slice (workspace) boundaries.
    state_checkpoint_interval = None

    def __init__(
        self,
        workspaces: list[str],
        skip_forks: bool = True,
        start_date: Optional[str] = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(**kwargs)
        self._workspaces = workspaces
        self._skip_forks = skip_forks
        self._start_date = _normalize_start_date(start_date)
        self._stop_pagination: bool = False

    def stream_slices(
        self,
        sync_mode: Optional[SyncMode] = None,
        cursor_field: Optional[list] = None,
        stream_state: Optional[Mapping[str, Any]] = None,
    ) -> Iterable[Optional[Mapping[str, Any]]]:
        state = stream_state or {}
        logger.info(
            f"repositories: {len(self._workspaces)} workspaces to fetch "
            f"(skip_forks={self._skip_forks})"
        )
        for workspace in self._workspaces:
            cursor = (state.get(workspace, {}) or {}).get(self.cursor_field, "") or ""
            self._stop_pagination = False
            logger.info(
                f"repositories: starting workspace '{workspace}' cursor={cursor or '<none>'}"
            )
            yield {"workspace": workspace, "cursor_value": cursor}

    def _path(self, stream_slice: Optional[Mapping[str, Any]] = None) -> str:
        workspace = (stream_slice or {}).get("workspace")
        if not workspace:
            raise ValueError("repositories stream_slice requires 'workspace'")
        return f"repositories/{workspace}"

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
            "sort": "-updated_on",
        }
        # Cursor wins; start_date fallback applies only on first run (no cursor).
        cursor = (stream_slice or {}).get("cursor_value", "") or ""
        q_date = cursor or self._start_date
        if q_date:
            params["q"] = f'updated_on>"{q_date}"'
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
        workspace = s.get("workspace", "")
        cursor_value = s.get("cursor_value", "") or ""
        skipped = 0
        emitted = 0
        for repo in self._iter_values(response):
            if self._skip_forks and repo.get("parent"):
                skipped += 1
                continue
            updated_on = repo.get("updated_on", "") or ""
            if cursor_value and updated_on and updated_on <= cursor_value:
                self._stop_pagination = True
                logger.info(
                    f"repositories: {workspace} cursor early-exit at "
                    f"updated_on={updated_on} cursor={cursor_value}"
                )
                return
            emitted += 1

            slug = repo.get("slug", "")
            project = repo.get("project") or {}
            mainbranch = (repo.get("mainbranch") or {}).get("name", "")

            record = {
                "unique_key": _make_unique_key(self._tenant_id, self._source_id, workspace, slug),
                "workspace": workspace,
                "slug": slug,
                "name": repo.get("name"),
                "full_name": repo.get("full_name"),
                "uuid": repo.get("uuid"),
                "is_private": repo.get("is_private"),
                "description": repo.get("description"),
                "language": repo.get("language"),
                "size": repo.get("size"),
                "created_on": repo.get("created_on"),
                "updated_on": updated_on,
                "has_issues": repo.get("has_issues"),
                "has_wiki": repo.get("has_wiki"),
                "mainbranch_name": mainbranch,
                "project_key": project.get("key"),
                "project_name": project.get("name"),
            }
            yield self._envelope(record)

        logger.info(
            f"repositories: workspace={workspace} emitted={emitted} "
            f"skipped_forks={skipped}"
        )

    def get_updated_state(
        self,
        current_stream_state: MutableMapping[str, Any],
        latest_record: Mapping[str, Any],
    ) -> MutableMapping[str, Any]:
        workspace = latest_record.get("workspace", "")
        if not workspace:
            return current_stream_state
        updated_on = latest_record.get(self.cursor_field, "") or ""
        if not updated_on:
            return current_stream_state
        entry = dict(current_stream_state.get(workspace, {}) or {})
        prev = entry.get(self.cursor_field, "") or ""
        if updated_on > prev:
            entry[self.cursor_field] = updated_on
            current_stream_state[workspace] = entry
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
                "workspace": {"type": "string"},
                "slug": {"type": ["null", "string"]},
                "name": {"type": ["null", "string"]},
                "full_name": {"type": ["null", "string"]},
                "uuid": {"type": ["null", "string"]},
                "is_private": {"type": ["null", "boolean"]},
                "description": {"type": ["null", "string"]},
                "language": {"type": ["null", "string"]},
                "size": {"type": ["null", "integer"]},
                "created_on": {"type": ["null", "string"]},
                "updated_on": {"type": ["null", "string"]},
                "has_issues": {"type": ["null", "boolean"]},
                "has_wiki": {"type": ["null", "boolean"]},
                "mainbranch_name": {"type": ["null", "string"]},
                "project_key": {"type": ["null", "string"]},
                "project_name": {"type": ["null", "string"]},
            },
        }
