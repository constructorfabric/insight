from __future__ import annotations

from collections.abc import Mapping
from typing import Any
from urllib.parse import quote

from source_gitlab.streams.base import ScopedGitlabStream


class ProjectsStream(ScopedGitlabStream):
    name = "projects"

    def _path(self, *, stream_slice: Mapping[str, Any] | None) -> str:
        slice_ = stream_slice or {"mode": "instance"}
        mode = slice_["mode"]
        if mode == "group":
            return f"groups/{quote(slice_['group'], safe='')}/projects"
        if mode == "project":
            return f"projects/{quote(slice_['project'], safe='')}"
        return "projects"

    def _initial_params(
        self, stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        slice_ = stream_slice or {"mode": "instance"}
        mode = slice_["mode"]
        if mode == "project":
            return {"statistics": "true"}
        if mode == "group":
            return {
                "per_page": self.page_size,
                "include_subgroups": "true",
                "statistics": "true",
            }
        return {
            "per_page": self.page_size,
            "pagination": "keyset",
            "order_by": "id",
            "sort": "asc",
            "statistics": "true",
        }

    def _record_key(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> list[str]:
        return [str(record["id"])]

    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        namespace = record.get("namespace") or {}
        statistics = record.get("statistics") or {}
        return {
            "id": record.get("id"),
            "name": record.get("name"),
            "path": record.get("path"),
            "path_with_namespace": record.get("path_with_namespace"),
            "description": record.get("description"),
            "default_branch": record.get("default_branch"),
            "visibility": record.get("visibility"),
            "archived": record.get("archived"),
            "empty_repo": record.get("empty_repo"),
            "created_at": record.get("created_at"),
            "last_activity_at": record.get("last_activity_at"),
            "web_url": record.get("web_url"),
            "namespace_id": namespace.get("id"),
            "namespace_full_path": namespace.get("full_path"),
            "statistics_commit_count": statistics.get("commit_count"),
            "statistics_repository_size": statistics.get("repository_size"),
        }
