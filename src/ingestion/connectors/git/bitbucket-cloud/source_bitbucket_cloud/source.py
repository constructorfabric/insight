"""Bitbucket Cloud Airbyte source connector (CDK-native)."""

import json
import logging
import sys
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from airbyte_cdk.sources import AbstractSource
from airbyte_cdk.sources.streams import Stream

from source_bitbucket_cloud.client import BitbucketApiError, BitbucketClient, RepositoryCatalog
from source_bitbucket_cloud.streams.branches import BranchesStream
from source_bitbucket_cloud.streams.commit_branch_reachability import CommitBranchReachabilityStream
from source_bitbucket_cloud.streams.commits import CommitsStream
from source_bitbucket_cloud.streams.file_changes import FileChangesStream
from source_bitbucket_cloud.streams.metric_events import (
    DeploymentsStream,
    EnvironmentsStream,
    IssueChangesStream,
    IssueCommentsStream,
    IssuesStream,
    PipelinesStream,
    PipelineStepsStream,
    PipelineStepTestReportsStream,
    PRTasksStream,
    TagsStream,
)
from source_bitbucket_cloud.streams.pr_activity import PRActivityStream
from source_bitbucket_cloud.streams.pr_comments import PRCommentsStream
from source_bitbucket_cloud.streams.pr_commits import PRCommitsStream
from source_bitbucket_cloud.streams.pr_diffstat import PRDiffstatStream
from source_bitbucket_cloud.streams.pull_requests import PullRequestsStream
from source_bitbucket_cloud.streams.repositories import RepositoriesStream

_logger = logging.getLogger("airbyte")


class SourceBitbucketCloud(AbstractSource):

    def spec(self, logger: Any) -> Mapping[str, Any]:
        from airbyte_cdk.models import ConnectorSpecification

        spec_path = Path(__file__).parent / "spec.json"
        return ConnectorSpecification(**json.loads(spec_path.read_text()))

    def check_connection(
        self, logger: Any, config: Mapping[str, Any]
    ) -> tuple[bool, Any | None]:
        token = config["bitbucket_token"]
        username = config.get("bitbucket_username", "")
        workspaces = config.get("bitbucket_workspaces", [])
        if not workspaces:
            return False, (
                "bitbucket_workspaces is empty — configure at least one "
                "workspace slug"
            )
        logger.info(
            f"check_connection: workspaces={workspaces} "
            f"username={'set' if username else 'unset'} token={'set' if token else 'unset'}"
        )
        try:
            client = BitbucketClient(token, username, config.get("bitbucket_api_base_url"))
            for workspace in workspaces:
                logger.info(f"check_connection: probing workspace '{workspace}'")
                client.request(
                    "GET",
                    f"repositories/{workspace}",
                    params={"pagelen": "1"},
                )
            logger.info("check_connection: OK for all workspaces")
            return True, None
        except BitbucketApiError as exc:
            if exc.status_code == 401:
                return False, "Authentication failed: invalid or expired token"
            if exc.status_code == 404:
                return False, "Workspace not found or not accessible with this token"
            if exc.status_code == 403:
                return False, "Token lacks permission to access the configured workspace"
            return False, str(exc)
        except Exception as exc:
            logger.exception("check_connection: request failed")
            return False, f"Bitbucket API request failed: {exc}"

    def streams(self, config: Mapping[str, Any]) -> list[Stream]:
        client = BitbucketClient(
            config["bitbucket_token"],
            config.get("bitbucket_username", ""),
            config.get("bitbucket_api_base_url"),
        )
        catalog = RepositoryCatalog(
            client,
            config["bitbucket_workspaces"],
            config.get("bitbucket_skip_forks", True),
        )
        shared = {
            "token": config["bitbucket_token"],
            "username": config.get("bitbucket_username", ""),
            "tenant_id": config["insight_tenant_id"],
            "source_id": config["insight_source_id"],
            "workspaces": config["bitbucket_workspaces"],
            "skip_forks": config.get("bitbucket_skip_forks", True),
            "start_date": config.get("bitbucket_start_date"),
            "client": client,
            "catalog": catalog,
        }

        repos = RepositoriesStream(**shared)
        branches = BranchesStream(**shared)
        commits = CommitsStream(**shared)
        commit_branch_reachability = CommitBranchReachabilityStream(**shared)
        file_changes = FileChangesStream(**shared)
        prs = PullRequestsStream(**shared)
        pr_diffstat = PRDiffstatStream(**shared)
        pr_activity = PRActivityStream(**shared)
        pr_comments = PRCommentsStream(**shared)
        pr_commits = PRCommitsStream(**shared)
        pipelines = PipelinesStream(**shared)
        pipeline_steps = PipelineStepsStream(**shared)
        pipeline_step_test_reports = PipelineStepTestReportsStream(**shared)
        deployments = DeploymentsStream(**shared)
        environments = EnvironmentsStream(**shared)
        tags = TagsStream(**shared)
        issues = IssuesStream(**shared)
        issue_comments = IssueCommentsStream(**shared)
        issue_changes = IssueChangesStream(**shared)
        pr_tasks = PRTasksStream(**shared)

        _logger.info(
            f"streams: wired 20 streams (workspaces={shared['workspaces']} "
            f"start_date={shared['start_date']} skip_forks={shared['skip_forks']})"
        )
        return [
            repos,
            branches,
            prs,
            pr_diffstat,
            pr_activity,
            pr_tasks,
            pr_comments,
            pr_commits,
            pipelines,
            pipeline_steps,
            pipeline_step_test_reports,
            deployments,
            environments,
            tags,
            issues,
            issue_comments,
            issue_changes,
            commits,
            commit_branch_reachability,
            file_changes,
        ]


def main() -> None:
    """CLI entry-point (source-bitbucket-cloud-insight)."""
    source = SourceBitbucketCloud()
    from airbyte_cdk.entrypoint import launch

    launch(source, sys.argv[1:])


if __name__ == "__main__":
    main()
