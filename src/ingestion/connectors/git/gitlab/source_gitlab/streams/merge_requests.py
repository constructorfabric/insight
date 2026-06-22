from __future__ import annotations

import json
from collections.abc import Mapping
from typing import Any

from source_gitlab.streams.base import MAX_BODY_CHARS, MAX_TITLE_CHARS, trim_text
from source_gitlab.streams.scope import ScopeUpdatedAtStream


class MergeRequestsStream(ScopeUpdatedAtStream):
    name = "merge_requests"
    resource = "merge_requests"

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
        merged_by = record.get("merged_by") or {}
        milestone = record.get("milestone") or {}
        assignees = record.get("assignees") or []
        reviewers = record.get("reviewers") or []
        return {
            "project_id": record.get("project_id"),
            "iid": record.get("iid"),
            "id": record.get("id"),
            "title": title,
            "title_truncated": title_truncated,
            "description": description,
            "description_truncated": description_truncated,
            "state": record.get("state"),
            "draft": record.get("draft"),
            "author_id": author.get("id"),
            "author_username": author.get("username"),
            "merged_by_id": merged_by.get("id"),
            "merged_by_username": merged_by.get("username"),
            "source_branch": record.get("source_branch"),
            "target_branch": record.get("target_branch"),
            "created_at": record.get("created_at"),
            "updated_at": record.get("updated_at"),
            "merged_at": record.get("merged_at"),
            "closed_at": record.get("closed_at"),
            "sha": record.get("sha"),
            "merge_commit_sha": record.get("merge_commit_sha"),
            "squash_commit_sha": record.get("squash_commit_sha"),
            "squash": record.get("squash"),
            "merge_status": record.get("merge_status"),
            "user_notes_count": record.get("user_notes_count"),
            "milestone_id": milestone.get("id"),
            "assignee_ids": json.dumps([a.get("id") for a in assignees]),
            "reviewer_ids": json.dumps([r.get("id") for r in reviewers]),
            "labels": json.dumps(record.get("labels") or []),
        }
