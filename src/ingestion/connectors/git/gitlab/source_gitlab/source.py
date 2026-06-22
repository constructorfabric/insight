from __future__ import annotations

import atexit
import json
import sys
from collections.abc import Mapping
from pathlib import Path
from typing import Any, TypedDict

import requests
from airbyte_cdk.models import ConnectorSpecification
from airbyte_cdk.sources import AbstractSource
from airbyte_cdk.sources.streams import Stream

from source_gitlab.client import GitlabClient
from source_gitlab.config import GitlabConfig
from source_gitlab.streams.branches import BranchesStream
from source_gitlab.streams.commits import CommitsStream
from source_gitlab.streams.concurrency import RequestGate
from source_gitlab.streams.file_changes import CommitFileChangesStream
from source_gitlab.streams.issues import IssuesStream
from source_gitlab.streams.merge_request_approvals import MergeRequestApprovalsStream
from source_gitlab.streams.merge_request_commits import MergeRequestCommitsStream
from source_gitlab.streams.merge_request_discussions import (
    MergeRequestDiscussionsStream,
)
from source_gitlab.streams.merge_request_notes import MergeRequestNotesStream
from source_gitlab.streams.merge_request_state_events import (
    MergeRequestStateEventsStream,
)
from source_gitlab.streams.merge_requests import MergeRequestsStream
from source_gitlab.streams.projects import ProjectsStream
from source_gitlab.streams.users import UsersStream


class _Connection(TypedDict):
    base_url: str
    token: str
    tenant_id: str
    source_id: str


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
            except requests.RequestException as exc:
                logger.warning(
                    f"Whole-instance mode: could not verify token admin status "
                    f"({type(exc).__name__}); coverage may be limited to the "
                    f"projects and users this token can access. Use an admin "
                    f"token for full coverage, or set gitlab_groups."
                )
            else:
                if not me.get("is_admin"):
                    logger.warning(
                        "Whole-instance mode with a non-admin token: coverage is "
                        "limited to projects and users this token can access. Use "
                        "an admin token for full coverage, or set gitlab_groups."
                    )
        return True, None

    def streams(self, config: Mapping[str, Any]) -> list[Stream]:
        cfg = GitlabConfig.parse(config)
        shared: _Connection = {
            "base_url": cfg.base_url,
            "token": cfg.token,
            "tenant_id": cfg.tenant_id,
            "source_id": cfg.source_id,
        }
        groups, projects_cfg, start = cfg.groups, cfg.projects, cfg.start_date
        gate = RequestGate(cfg.max_workers)
        atexit.register(gate.shutdown)
        projects = ProjectsStream(groups=groups, projects=projects_cfg, **shared)
        branches = BranchesStream(parent=projects, gate=gate, **shared)
        return [
            projects,
            UsersStream(groups=groups, projects=projects_cfg, **shared),
            branches,
            CommitsStream(
                parent=projects, branches=branches, gate=gate, start_date=start, **shared
            ),
            CommitFileChangesStream(
                parent=projects, branches=branches, gate=gate, start_date=start, **shared
            ),
            MergeRequestsStream(
                parent=projects, gate=gate, groups=groups, projects=projects_cfg,
                start_date=start, **shared
            ),
            MergeRequestCommitsStream(
                parent=projects, gate=gate, groups=groups, projects=projects_cfg,
                start_date=start, **shared
            ),
            MergeRequestNotesStream(
                parent=projects, gate=gate, groups=groups, projects=projects_cfg,
                start_date=start, **shared
            ),
            MergeRequestDiscussionsStream(
                parent=projects, gate=gate, groups=groups, projects=projects_cfg,
                start_date=start, **shared
            ),
            MergeRequestApprovalsStream(
                parent=projects, gate=gate, groups=groups, projects=projects_cfg,
                start_date=start, **shared
            ),
            MergeRequestStateEventsStream(
                parent=projects, gate=gate, groups=groups, projects=projects_cfg,
                start_date=start, **shared
            ),
            IssuesStream(
                parent=projects, gate=gate, groups=groups, projects=projects_cfg,
                start_date=start, **shared
            ),
        ]


def main() -> None:
    from airbyte_cdk.entrypoint import launch

    launch(SourceGitlab(), sys.argv[1:])


if __name__ == "__main__":
    main()
