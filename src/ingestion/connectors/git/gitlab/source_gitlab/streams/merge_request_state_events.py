from __future__ import annotations

from collections.abc import Mapping
from typing import Any

from source_gitlab.streams.merge_request_child import MergeRequestChildStream


class MergeRequestStateEventsStream(MergeRequestChildStream):
    name = "merge_request_state_events"

    def _path(self, *, stream_slice: Mapping[str, Any] | None) -> str:
        s = stream_slice or {}
        return f"projects/{s['project_id']}/merge_requests/{s['mr_iid']}/resource_state_events"

    def _record_key(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> list[str]:
        s = stream_slice or {}
        return [str(s["project_id"]), str(record["id"])]

    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        s = stream_slice or {}
        user = record.get("user") or {}
        return {
            "project_id": s["project_id"],
            "mr_iid": s["mr_iid"],
            "mr_updated_at": s.get("mr_updated_at"),
            "id": record.get("id"),
            "user_id": user.get("id"),
            "user_username": user.get("username"),
            "state": record.get("state"),
            "created_at": record.get("created_at"),
        }
