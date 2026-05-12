"""Bitbucket Cloud Airbyte source connector (CDK-native)."""

import json
import logging
import sys
from pathlib import Path
from typing import Any, List, Mapping, Optional, Tuple

import requests
from airbyte_cdk.sources import AbstractSource
from airbyte_cdk.sources.streams import Stream

_logger = logging.getLogger("airbyte")

from source_bitbucket_cloud.auth import auth_headers
from source_bitbucket_cloud.streams.branches import BranchesStream
from source_bitbucket_cloud.streams.commits import CommitsStream
from source_bitbucket_cloud.streams.file_changes import FileChangesStream
from source_bitbucket_cloud.streams.pr_comments import PRCommentsStream
from source_bitbucket_cloud.streams.pr_commits import PRCommitsStream
from source_bitbucket_cloud.streams.pull_requests import PullRequestsStream
from source_bitbucket_cloud.streams.repositories import RepositoriesStream


class SourceBitbucketCloud(AbstractSource):

    def spec(self, logger: Any) -> Mapping[str, Any]:
        from airbyte_cdk.models import ConnectorSpecification

        spec_path = Path(__file__).parent / "spec.json"
        return ConnectorSpecification(**json.loads(spec_path.read_text()))

    def check_connection(
        self, logger: Any, config: Mapping[str, Any]
    ) -> Tuple[bool, Optional[Any]]:
        token = config["bitbucket_token"]
        username = config.get("bitbucket_username", "")
        workspaces = config.get("bitbucket_workspaces", [])
        if not workspaces:
            return False, (
                "bitbucket_workspaces is empty — configure at least one "
                "workspace slug"
            )
        headers = auth_headers(token, username)

        logger.info(
            f"check_connection: workspaces={workspaces} "
            f"username={'set' if username else 'unset'} token={'set' if token else 'unset'}"
        )
        try:
            for workspace in workspaces:
                logger.info(f"check_connection: probing workspace '{workspace}'")
                resp = requests.get(
                    f"https://api.bitbucket.org/2.0/repositories/{workspace}?pagelen=1",
                    headers=headers,
                    timeout=10,
                )
                logger.info(
                    f"check_connection: workspace='{workspace}' status={resp.status_code}"
                )
                if resp.status_code == 401:
                    return False, "Authentication failed: invalid or expired token"
                if resp.status_code == 404:
                    return False, (
                        f"Workspace '{workspace}' not found or not accessible "
                        f"with this token"
                    )
                if resp.status_code == 403:
                    return False, (
                        f"Token lacks permission to access workspace '{workspace}'"
                    )
                if resp.status_code != 200:
                    return False, (
                        f"Failed to access workspace '{workspace}' "
                        f"({resp.status_code}): {resp.text[:200]}"
                    )
            logger.info("check_connection: OK for all workspaces")
            return True, None
        except requests.RequestException as exc:
            logger.exception("check_connection: request failed")
            return False, f"Bitbucket API request failed: {exc}"

    def streams(self, config: Mapping[str, Any]) -> List[Stream]:
        shared = {
            "token": config["bitbucket_token"],
            "username": config.get("bitbucket_username", ""),
            "tenant_id": config["insight_tenant_id"],
            "source_id": config["insight_source_id"],
        }
        workspaces = config["bitbucket_workspaces"]
        start_date = config.get("bitbucket_start_date")
        skip_forks = config.get("bitbucket_skip_forks", True)

        repos = RepositoriesStream(
            workspaces=workspaces, skip_forks=skip_forks, start_date=start_date, **shared,
        )
        branches = BranchesStream(parent=repos, start_date=start_date, **shared)
        commits = CommitsStream(parent=branches, start_date=start_date, **shared)
        file_changes = FileChangesStream(parent=commits, start_date=start_date, **shared)
        prs = PullRequestsStream(parent=repos, start_date=start_date, **shared)
        pr_comments = PRCommentsStream(parent=prs, **shared)
        pr_commits = PRCommitsStream(parent=prs, **shared)

        _logger.info(
            f"streams: wired 7 streams (workspaces={workspaces} "
            f"start_date={start_date} skip_forks={skip_forks})"
        )
        # Order: cheap → expensive. If pod dies, cheaper streams have landed.
        return [repos, branches, prs, pr_comments, pr_commits, commits, file_changes]


def main() -> None:
    """CLI entry-point (source-bitbucket-cloud-insight)."""
    source = SourceBitbucketCloud()
    from airbyte_cdk.entrypoint import launch

    launch(source, sys.argv[1:])


if __name__ == "__main__":
    main()
