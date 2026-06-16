from __future__ import annotations

import json
import sys
from collections.abc import Mapping
from pathlib import Path
from typing import Any

import requests
from airbyte_cdk.models import ConnectorSpecification
from airbyte_cdk.sources import AbstractSource
from airbyte_cdk.sources.streams import Stream

from source_gitlab.client import GitlabClient
from source_gitlab.config import GitlabConfig
from source_gitlab.streams.branches import BranchesStream
from source_gitlab.streams.commits import CommitsStream
from source_gitlab.streams.file_changes import CommitFileChangesStream
from source_gitlab.streams.merge_requests import MergeRequestsStream
from source_gitlab.streams.projects import ProjectsStream
from source_gitlab.streams.users import UsersStream


class SourceGitlab(AbstractSource):
    def spec(self, logger: Any) -> ConnectorSpecification:
        spec_path = Path(__file__).parent / "spec.json"
        return ConnectorSpecification(**json.loads(spec_path.read_text()))

    def check_connection(
        self, logger: Any, config: Mapping[str, Any]
    ) -> tuple[bool, Any | None]:
        cfg = GitlabConfig.parse(config)
        client = GitlabClient(cfg.api_base, cfg.token)
        try:
            version = client.version()
        except requests.RequestException as exc:
            return False, f"GitLab API unreachable or token invalid: {exc}"
        logger.info(
            f"Connected to GitLab {version.get('version')} "
            f"({version.get('revision')})"
        )
        for group in cfg.groups:
            ok, err = client.check_group(group)
            if not ok:
                return False, err
        for project in cfg.projects:
            ok, err = client.check_project(project)
            if not ok:
                return False, err
        if not cfg.groups and not cfg.projects:
            try:
                me = client.current_user()
            except requests.RequestException:
                me = {}
            if not me.get("is_admin"):
                logger.warning(
                    "Whole-instance mode with a non-admin token: coverage is "
                    "limited to projects and users this token can access. Use an "
                    "admin token for full coverage, or set gitlab_groups."
                )
        return True, None

    def streams(self, config: Mapping[str, Any]) -> list[Stream]:
        cfg = GitlabConfig.parse(config)
        shared = {
            "base_url": cfg.base_url,
            "token": cfg.token,
            "tenant_id": cfg.tenant_id,
            "source_id": cfg.source_id,
        }
        projects = ProjectsStream(groups=cfg.groups, projects=cfg.projects, **shared)
        branches = BranchesStream(parent=projects, **shared)
        return [
            projects,
            UsersStream(groups=cfg.groups, projects=cfg.projects, **shared),
            branches,
            CommitsStream(
                parent=projects, branches=branches, start_date=cfg.start_date, **shared
            ),
            CommitFileChangesStream(
                parent=projects, branches=branches, start_date=cfg.start_date, **shared
            ),
            MergeRequestsStream(parent=projects, **shared),
        ]


def main() -> None:
    from airbyte_cdk.entrypoint import launch

    launch(SourceGitlab(), sys.argv[1:])


if __name__ == "__main__":
    main()
