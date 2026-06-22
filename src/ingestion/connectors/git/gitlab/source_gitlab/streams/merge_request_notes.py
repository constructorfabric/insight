from __future__ import annotations

from collections.abc import Mapping
from typing import Any

from source_gitlab.streams.base import MAX_BODY_CHARS, trim_text
from source_gitlab.streams.merge_request_child import MergeRequestChildStream


class MergeRequestNotesStream(MergeRequestChildStream):
    name = "merge_request_notes"

    def _path(self, *, stream_slice: Mapping[str, Any] | None) -> str:
        s = stream_slice or {}
        return f"projects/{s['project_id']}/merge_requests/{s['mr_iid']}/notes"

    def _record_key(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> list[str]:
        s = stream_slice or {}
        return [str(s["project_id"]), str(record["id"])]

    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        s = stream_slice or {}
        body, body_truncated = trim_text(record.get("body"), MAX_BODY_CHARS)
        author = record.get("author") or {}
        position = record.get("position") or {}
        return {
            "project_id": s["project_id"],
            "mr_iid": s["mr_iid"],
            "mr_updated_at": s.get("mr_updated_at"),
            "id": record.get("id"),
            "body": body,
            "body_truncated": body_truncated,
            "author_id": author.get("id"),
            "author_username": author.get("username"),
            "created_at": record.get("created_at"),
            "updated_at": record.get("updated_at"),
            "system": record.get("system"),
            "resolvable": record.get("resolvable"),
            "resolved": record.get("resolved"),
            "resolved_by_id": (record.get("resolved_by") or {}).get("id"),
            "noteable_type": record.get("noteable_type"),
            "position_new_path": position.get("new_path"),
            "position_new_line": position.get("new_line"),
        }
