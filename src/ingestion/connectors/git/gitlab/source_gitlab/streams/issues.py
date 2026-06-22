from __future__ import annotations

import json
from collections.abc import Mapping
from typing import Any

from source_gitlab.streams.base import MAX_BODY_CHARS, MAX_TITLE_CHARS, trim_text
from source_gitlab.streams.scope import ScopeUpdatedAtStream


class IssuesStream(ScopeUpdatedAtStream):
    name = "issues"
    resource = "issues"

    def _record_key(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> list[str]:
        return [str(record["project_id"]), str(record["iid"])]

    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        title, title_truncated = trim_text(record.get("title"), MAX_TITLE_CHARS)
        description, description_truncated = trim_text(
            record.get("description"), MAX_BODY_CHARS
        )
        author = record.get("author") or {}
        milestone = record.get("milestone") or {}
        assignees = record.get("assignees") or []
        return {
            "project_id": record.get("project_id"),
            "iid": record.get("iid"),
            "id": record.get("id"),
            "title": title,
            "title_truncated": title_truncated,
            "description": description,
            "description_truncated": description_truncated,
            "state": record.get("state"),
            "author_id": author.get("id"),
            "author_username": author.get("username"),
            "created_at": record.get("created_at"),
            "updated_at": record.get("updated_at"),
            "closed_at": record.get("closed_at"),
            "closed_by_id": (record.get("closed_by") or {}).get("id"),
            "milestone_id": milestone.get("id"),
            "user_notes_count": record.get("user_notes_count"),
            "assignee_ids": json.dumps([a.get("id") for a in assignees]),
            "labels": json.dumps(record.get("labels") or []),
        }
