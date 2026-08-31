"""Repositories, as each vendor's connector reports them, into bronze.

The class model builds silver from these, so the vendor split — and the
`default_branch` each one carries — is exercised rather than asserted.
"""

from __future__ import annotations

from collections.abc import Sequence
from typing import TYPE_CHECKING

from ..profiles import Person
from .base import anchor_datetime, deterministic_int, deterministic_uuid
from .ci_topology import Repo, repo_grid
from .insert import bulk_insert, truncate

if TYPE_CHECKING:
    import clickhouse_connect.driver.client

GITHUB_SOURCE_ID = deterministic_uuid("git.source", "insight_github")
GITLAB_SOURCE_ID = deterministic_uuid("git.source", "insight_gitlab")
BITBUCKET_SOURCE_ID = deterministic_uuid("git.source", "insight_bitbucket_cloud")


def _envelope(source_id: str, tenant_uuid: str) -> dict[str, object]:
    read_at = anchor_datetime()
    return {
        "_airbyte_extracted_at": read_at,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": tenant_uuid,
        "source_id": source_id,
        "collected_at": read_at.isoformat(),
    }


def _positioned(cols: list[str], fields: dict[str, object]) -> tuple[object, ...]:
    return tuple(fields.get(col) for col in cols)


def seed_github_repositories(
    client: clickhouse_connect.driver.client.Client, grid: Sequence[Repo], tenant_uuid: str
) -> int:
    truncate(client, "bronze_github", "repositories")
    truncate(client, "staging", "github__repositories")
    truncate(client, "silver", "class_git_repositories")
    cols = [
        "_airbyte_raw_id",
        "_airbyte_extracted_at",
        "_airbyte_meta",
        "_airbyte_generation_id",
        "unique_key",
        "tenant_id",
        "source_id",
        "data_source",
        "collected_at",
        "id",
        "full_name",
        "name",
        "org",
        "default_branch",
        "private",
        "language",
        "created_at",
        "updated_at",
        "pushed_at",
        "description",
    ]
    envelope = _envelope(GITHUB_SOURCE_ID, tenant_uuid)
    created = anchor_datetime().isoformat()
    rows = []
    for repo in (r for r in grid if r.vendor == "github"):
        org, _, name = repo.full_name.partition("/")
        repo_id = deterministic_int("git.repo", repo.full_name)
        rows.append(
            _positioned(
                cols,
                {
                    **envelope,
                    "_airbyte_raw_id": deterministic_uuid("git.repo.raw", repo.full_name),
                    "unique_key": f"{tenant_uuid}:{GITHUB_SOURCE_ID}:{repo_id}",
                    "data_source": "insight_github",
                    "id": repo_id,
                    "full_name": repo.full_name,
                    "name": name,
                    "org": org,
                    "default_branch": repo.default_branch,
                    "private": True,
                    "language": "Rust",
                    "created_at": created,
                    "updated_at": created,
                    "pushed_at": created,
                    "description": f"Synthetic repository {repo.full_name}",
                },
            )
        )
    return bulk_insert(client, "bronze_github", "repositories", cols, rows)


def seed_gitlab_projects(
    client: clickhouse_connect.driver.client.Client, grid: Sequence[Repo], tenant_uuid: str
) -> int:
    truncate(client, "bronze_gitlab", "projects")
    truncate(client, "staging", "gitlab__repositories")
    cols = [
        "_airbyte_raw_id",
        "_airbyte_extracted_at",
        "_airbyte_meta",
        "_airbyte_generation_id",
        "unique_key",
        "tenant_id",
        "source_id",
        "data_source",
        "collected_at",
        "id",
        "name",
        "path",
        "path_with_namespace",
        "description",
        "default_branch",
        "visibility",
        "created_at",
        "last_activity_at",
        "namespace_full_path",
    ]
    envelope = _envelope(GITLAB_SOURCE_ID, tenant_uuid)
    created = anchor_datetime().isoformat()
    rows = []
    for repo in (r for r in grid if r.vendor == "gitlab"):
        namespace, _, path = repo.full_name.partition("/")
        project_id = deterministic_int("git.repo", repo.full_name)
        rows.append(
            _positioned(
                cols,
                {
                    **envelope,
                    "_airbyte_raw_id": deterministic_uuid("git.repo.raw", repo.full_name),
                    "unique_key": f"{tenant_uuid}:{GITLAB_SOURCE_ID}:{project_id}",
                    "data_source": "insight_gitlab",
                    "id": project_id,
                    "name": path,
                    "path": path,
                    "path_with_namespace": repo.full_name,
                    "description": f"Synthetic project {repo.full_name}",
                    "default_branch": repo.default_branch,
                    "visibility": "private",
                    "created_at": created,
                    "last_activity_at": created,
                    "namespace_full_path": namespace,
                },
            )
        )
    return bulk_insert(client, "bronze_gitlab", "projects", cols, rows)


def seed_bitbucket_repositories(
    client: clickhouse_connect.driver.client.Client, grid: Sequence[Repo], tenant_uuid: str
) -> int:
    truncate(client, "bronze_bitbucket_cloud", "repositories")
    truncate(client, "staging", "bitbucket_cloud__repositories")
    cols = [
        "_airbyte_raw_id",
        "_airbyte_extracted_at",
        "_airbyte_meta",
        "_airbyte_generation_id",
        "unique_key",
        "tenant_id",
        "source_id",
        "data_source",
        "repository_uuid",
        "slug",
        "name",
        "full_name",
        "is_private",
        "language",
        "created_on",
        "updated_on",
        "description",
        "default_branch",
        "workspace_slug",
    ]
    envelope = _envelope(BITBUCKET_SOURCE_ID, tenant_uuid)
    created = anchor_datetime().isoformat()
    rows = []
    for repo in (r for r in grid if r.vendor == "bitbucket"):
        workspace, _, slug = repo.full_name.partition("/")
        repo_uuid = deterministic_uuid("git.repo.uuid", repo.full_name)
        rows.append(
            _positioned(
                cols,
                {
                    **envelope,
                    "_airbyte_raw_id": deterministic_uuid("git.repo.raw", repo.full_name),
                    "unique_key": f"{tenant_uuid}:{BITBUCKET_SOURCE_ID}:{repo_uuid}",
                    "data_source": "insight_bitbucket_cloud",
                    "repository_uuid": repo_uuid,
                    "slug": slug,
                    "name": slug,
                    "full_name": repo.full_name,
                    "is_private": True,
                    "language": "swift",
                    "created_on": created,
                    "updated_on": created,
                    "description": f"Synthetic repository {repo.full_name}",
                    "default_branch": repo.default_branch,
                    "workspace_slug": workspace,
                },
            )
        )
    return bulk_insert(client, "bronze_bitbucket_cloud", "repositories", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
) -> dict[str, int]:
    grid = repo_grid(roster)
    return {
        "bronze_github.repositories": seed_github_repositories(client, grid, tenant_uuid),
        "bronze_gitlab.projects": seed_gitlab_projects(client, grid, tenant_uuid),
        "bronze_bitbucket_cloud.repositories": seed_bitbucket_repositories(
            client, grid, tenant_uuid
        ),
    }
