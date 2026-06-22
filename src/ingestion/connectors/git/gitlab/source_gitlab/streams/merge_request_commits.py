from __future__ import annotations

from collections.abc import Mapping
from typing import Any

from source_gitlab.streams.base import MAX_BODY_CHARS, MAX_TITLE_CHARS, trim_text
from source_gitlab.streams.merge_request_child import MergeRequestChildStream


class MergeRequestCommitsStream(MergeRequestChildStream):
    name = "merge_request_commits"

    def _path(self, *, stream_slice: Mapping[str, Any] | None) -> str:
        s = stream_slice or {}
        return f"projects/{s['project_id']}/merge_requests/{s['mr_iid']}/commits"

    def _record_key(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> list[str]:
        s = stream_slice or {}
        return [str(s["project_id"]), str(s["mr_iid"]), str(record["id"])]

    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        s = stream_slice or {}
        message, message_truncated = trim_text(record.get("message"), MAX_BODY_CHARS)
        title, title_truncated = trim_text(record.get("title"), MAX_TITLE_CHARS)
        return {
            "project_id": s["project_id"],
            "mr_iid": s["mr_iid"],
            "mr_updated_at": s.get("mr_updated_at"),
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
        }
