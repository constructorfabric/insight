from __future__ import annotations

from typing import Any
from urllib.parse import quote

import requests


class GitlabClient:
    def __init__(self, api_base: str, token: str) -> None:
        self._api_base = api_base
        self._session = requests.Session()
        self._session.headers.update(
            {"PRIVATE-TOKEN": token, "Accept": "application/json"}
        )

    def version(self) -> dict[str, Any]:
        resp = self._session.get(f"{self._api_base}/version", timeout=15)
        resp.raise_for_status()
        return dict(resp.json())

    def current_user(self) -> dict[str, Any]:
        resp = self._session.get(f"{self._api_base}/user", timeout=15)
        resp.raise_for_status()
        return dict(resp.json())

    def check_group(self, group: str) -> tuple[bool, str | None]:
        encoded = quote(group, safe="")
        try:
            resp = self._session.get(
                f"{self._api_base}/groups/{encoded}", timeout=15
            )
        except requests.RequestException as exc:
            return False, f"Failed to reach group '{group}': {type(exc).__name__}"
        if resp.status_code == 404:
            return False, f"Group '{group}' not found or not accessible with this token"
        if resp.status_code in (401, 403):
            return False, f"Token lacks access to group '{group}' ({resp.status_code})"
        if resp.status_code != 200:
            return False, (
                f"Failed to access group '{group}' "
                f"({resp.status_code}): {resp.text[:200]}"
            )
        return True, None

    def check_project(self, project: str) -> tuple[bool, str | None]:
        encoded = quote(project, safe="")
        try:
            resp = self._session.get(
                f"{self._api_base}/projects/{encoded}", timeout=15
            )
        except requests.RequestException as exc:
            return False, f"Failed to reach project '{project}': {type(exc).__name__}"
        if resp.status_code == 404:
            return False, f"Project '{project}' not found or not accessible with this token"
        if resp.status_code in (401, 403):
            return False, f"Token lacks access to project '{project}' ({resp.status_code})"
        if resp.status_code != 200:
            return False, (
                f"Failed to access project '{project}' "
                f"({resp.status_code}): {resp.text[:200]}"
            )
        return True, None
