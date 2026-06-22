from __future__ import annotations

import json
from collections.abc import Mapping
from typing import Any

from source_gitlab.streams.merge_request_child import MergeRequestChildStream


class MergeRequestDiscussionsStream(MergeRequestChildStream):
    name = "merge_request_discussions"

    def _path(self, *, stream_slice: Mapping[str, Any] | None) -> str:
        s = stream_slice or {}
        return f"projects/{s['project_id']}/merge_requests/{s['mr_iid']}/discussions"

    def _record_key(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> list[str]:
        s = stream_slice or {}
        return [str(s["project_id"]), str(s["mr_iid"]), str(record["id"])]

    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        s = stream_slice or {}
        notes = record.get("notes") or []
        return {
            "project_id": s["project_id"],
            "mr_iid": s["mr_iid"],
            "mr_updated_at": s.get("mr_updated_at"),
            "discussion_id": record.get("id"),
            "individual_note": record.get("individual_note"),
            "note_ids": json.dumps([n.get("id") for n in notes]),
        }
