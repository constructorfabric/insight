from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from source_bitbucket_cloud.client import BitbucketApiError
from source_bitbucket_cloud.streams.base import BitbucketIncrementalStream, schema, unique_key
from source_bitbucket_cloud.streams.git_ranges import CommitRangeMixin


class CommitBranchReachabilityStream(CommitRangeMixin, BitbucketIncrementalStream):
    name = "commit_branch_reachability"
    cursor_field = "branch_head_sha"

    def repository_records(self, repo, bucket_id: int) -> Iterable[Mapping[str, Any]]:
        del bucket_id
        prior = self.repository_state(repo)
        branches, current_heads = self.branch_snapshot(repo)
        previous_heads = prior.get("heads") or {}
        branch_by_name = {branch.name: branch for branch in branches}
        for branch_name in sorted(set(current_heads) | set(previous_heads)):
            old_head = previous_heads.get(branch_name)
            new_head = current_heads.get(branch_name)
            if old_head == new_head:
                continue
            if new_head:
                yield from self._changes(
                    repo,
                    branch_by_name[branch_name],
                    new_head,
                    old_head,
                    "added",
                )
            if old_head and new_head:
                yield from self._changes(
                    repo,
                    branch_by_name[branch_name],
                    old_head,
                    new_head,
                    "removed",
                )
            if old_head and not new_head:
                entity_key = unique_key(
                    self._tenant_id,
                    self._source_id,
                    repo.uuid,
                    branch_name,
                    old_head,
                    "deleted",
                )
                yield self.item(
                    entity_key=entity_key,
                    repository_uuid=repo.uuid,
                    workspace_uuid=repo.workspace_uuid,
                    workspace=repo.workspace,
                    repo_slug=repo.slug,
                    branch_name=branch_name,
                    branch_head_sha=old_head,
                    default_branch_name=repo.mainbranch_name,
                    commit_sha=None,
                    committed_at=None,
                    reachability_action="branch_deleted",
                )
        self.commit_repository_state(repo, {"heads": current_heads})

    def _changes(self, repo, branch, include: str, exclude: str | None, action: str):
        try:
            commits = list(self._client.commits_between(repo, [include], [exclude] if exclude else []))
        except BitbucketApiError as exc:
            if exc.status_code != 404 or not exclude:
                raise
            if action == "added":
                commits = self._client.commits_between(repo, [include], [])
                yield from self._reachability_records(repo, branch, include, "reset", commits)
                return
            entity_key = unique_key(
                self._tenant_id,
                self._source_id,
                repo.uuid,
                branch.name,
                include,
                "removal_unavailable",
            )
            yield self.item(
                entity_key=entity_key,
                repository_uuid=repo.uuid,
                workspace_uuid=repo.workspace_uuid,
                workspace=repo.workspace,
                repo_slug=repo.slug,
                branch_name=branch.name,
                branch_head_sha=include,
                default_branch_name=repo.mainbranch_name,
                commit_sha=None,
                committed_at=None,
                reachability_action="removal_unavailable",
            )
            return
        yield from self._reachability_records(repo, branch, include, action, commits)

    def _reachability_records(self, repo, branch, head, action, commits):
        for commit in commits:
            committed_at = commit.get("date")
            if self._start_date and committed_at and str(committed_at)[:10] < self._start_date:
                continue
            sha = str(commit.get("hash") or "")
            if not sha:
                continue
            entity_key = unique_key(
                self._tenant_id,
                self._source_id,
                repo.uuid,
                branch.name,
                head,
                action,
                sha,
            )
            yield self.item(
                entity_key=entity_key,
                repository_uuid=repo.uuid,
                workspace_uuid=repo.workspace_uuid,
                workspace=repo.workspace,
                repo_slug=repo.slug,
                branch_name=branch.name,
                branch_head_sha=head,
                default_branch_name=repo.mainbranch_name,
                commit_sha=sha,
                committed_at=committed_at,
                reachability_action=action,
            )

    def get_json_schema(self) -> Mapping[str, Any]:
        nullable_string = {"type": ["null", "string"]}
        return schema(
            {
                "workspace": nullable_string,
                "repo_slug": nullable_string,
                "branch_name": nullable_string,
                "branch_head_sha": nullable_string,
                "default_branch_name": nullable_string,
                "commit_sha": nullable_string,
                "committed_at": nullable_string,
                "reachability_action": nullable_string,
            }
        )
