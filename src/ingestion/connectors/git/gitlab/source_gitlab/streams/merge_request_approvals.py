from __future__ import annotations

import json
from collections.abc import Mapping
from typing import Any

from source_gitlab.streams.merge_request_child import MergeRequestChildStream


class MergeRequestApprovalsStream(MergeRequestChildStream):
    name = "merge_request_approvals"
    skippable_statuses = frozenset({402, 404})

    def _path(self, *, stream_slice: Mapping[str, Any] | None) -> str:
        s = stream_slice or {}
        return f"projects/{s['project_id']}/merge_requests/{s['mr_iid']}/approvals"

    def _record_key(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> list[str]:
        s = stream_slice or {}
        return [str(s["project_id"]), str(s["mr_iid"])]

    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        s = stream_slice or {}
        approved_by = record.get("approved_by") or []
        return {
            "project_id": s["project_id"],
            "mr_iid": s["mr_iid"],
            "mr_updated_at": s.get("mr_updated_at"),
            "approvals_required": record.get("approvals_required"),
            "approvals_left": record.get("approvals_left"),
            "approved": record.get("approved"),
            "approved_by": json.dumps(
                [
                    {
                        "id": (a.get("user") or {}).get("id"),
                        "username": (a.get("user") or {}).get("username"),
                    }
                    for a in approved_by
                ]
            ),
        }
