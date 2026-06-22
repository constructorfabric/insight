from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from typing import Any

from source_gitlab.streams.timeutil import parse_iso, to_utc_z


def _require(config: Mapping[str, Any], key: str) -> str:
    value = config.get(key)
    if value is None:
        raise ValueError(f"Required config '{key}' is missing or null")
    return str(value)


@dataclass(frozen=True)
class GitlabConfig:
    tenant_id: str
    source_id: str
    base_url: str
    token: str
    groups: tuple[str, ...]
    projects: tuple[str, ...]
    start_date: str | None
    max_workers: int

    @property
    def api_base(self) -> str:
        return f"{self.base_url}/api/v4"

    @classmethod
    def parse(cls, config: Mapping[str, Any]) -> GitlabConfig:
        raw_start = config.get("gitlab_start_date")
        return cls(
            tenant_id=_require(config, "insight_tenant_id"),
            source_id=_require(config, "insight_source_id"),
            base_url=_require(config, "gitlab_url").rstrip("/"),
            token=_require(config, "gitlab_token"),
            groups=tuple(str(g) for g in (config.get("gitlab_groups") or ())),
            projects=tuple(str(p) for p in (config.get("gitlab_projects") or ())),
            start_date=to_utc_z(parse_iso(str(raw_start))) if raw_start else None,
            max_workers=max(1, min(32, int(config.get("gitlab_max_workers") or 8))),
        )
