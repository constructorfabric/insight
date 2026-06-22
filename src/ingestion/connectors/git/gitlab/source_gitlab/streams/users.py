from __future__ import annotations

from collections.abc import Mapping
from typing import Any
from urllib.parse import quote

from source_gitlab.streams.base import ScopedGitlabStream


class UsersStream(ScopedGitlabStream):
    name = "users"

    def _path(self, *, stream_slice: Mapping[str, Any] | None) -> str:
        slice_ = stream_slice or {"mode": "instance"}
        mode = slice_["mode"]
        if mode == "group":
            return f"groups/{quote(slice_['group'], safe='')}/members/all"
        if mode == "project":
            return f"projects/{quote(slice_['project'], safe='')}/members/all"
        return "users"

    def _initial_params(
        self, stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        slice_ = stream_slice or {"mode": "instance"}
        if slice_["mode"] == "instance":
            return {
                "per_page": self.page_size,
                "pagination": "keyset",
                "order_by": "id",
                "sort": "asc",
            }
        return {"per_page": self.page_size}

    def _record_key(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> list[str]:
        return [str(record["id"])]

    def _project(
        self, record: Mapping[str, Any], stream_slice: Mapping[str, Any] | None
    ) -> Mapping[str, Any]:
        return {
            "id": record.get("id"),
            "username": record.get("username"),
            "name": record.get("name"),
            "state": record.get("state"),
            "email": record.get("email"),
            "public_email": record.get("public_email"),
            "bot": record.get("bot"),
            "web_url": record.get("web_url"),
        }
