from __future__ import annotations

from collections.abc import Iterable, Iterator, Mapping
from itertools import chain
from typing import Any

from source_bitbucket_cloud.client import BitbucketApiError, BranchRef, RepositoryRef
from source_bitbucket_cloud.streams.base import BitbucketIncrementalStream, repo_scope, schema, unique_key
from source_bitbucket_cloud.streams.git_ranges import CommitRangeMixin


RANGE_PREFETCH = 500


class CommitBranchReachabilityStream(CommitRangeMixin, BitbucketIncrementalStream):
    name = "commit_branch_reachability"
    cursor_field = "branch_head_sha"

    def repository_records(self, repo, bucket_id: int) -> Iterable[Mapping[str, Any]]:
        del bucket_id
        if self.out_of_window(repo):
            return
        prior = self.repository_state(repo)
        repo_updated_on = str(repo.raw.get("updated_on") or "")
        if repo_updated_on and prior.get("repo_updated_on") == repo_updated_on:
            # The repository has not been pushed to since the last successful
            # pass (updated_on comes free with the workspace listing), so the
            # branch heads cannot have moved: skip the branch listing and the
            # range fetch entirely. This is what keeps the per-repository
            # request budget at zero for the idle majority of a large fleet.
            return
        branches, current_heads = self.branch_snapshot(repo)
        previous_heads = prior.get("heads") or {}
        if previous_heads and not current_heads:
            # Every stored branch would read as deleted, and a later listing
            # that finds them again emits no correction. An answer this
            # sweeping is not trusted: nothing is emitted, nothing advances.
            return
        branch_by_name = {branch.name: branch for branch in branches}
        unresolved: set[str] = set()
        for branch_name in sorted(set(current_heads) | set(previous_heads)):
            old_head = previous_heads.get(branch_name)
            new_head = current_heads.get(branch_name)
            if old_head == new_head:
                continue
            if new_head and not old_head and not self.head_in_window(branch_by_name[branch_name]):
                continue
            if new_head:
                yield from self._changes(
                    repo,
                    branch_by_name[branch_name],
                    new_head,
                    old_head,
                    "added",
                    unresolved,
                )
            if old_head and new_head:
                yield from self._changes(
                    repo,
                    branch_by_name[branch_name],
                    old_head,
                    new_head,
                    "removed",
                    unresolved,
                )
            if old_head and not new_head:
                entity_key = unique_key(
                    self._tenant_id,
                    self._source_id,
                    *repo_scope(repo),
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
        stored = {
            name: head for name, head in self.retained_heads(current_heads, previous_heads).items()
            if name not in unresolved
        }
        complete = self.complete_read(
            current_heads, unresolved, empty_confirmed=self.empty_listing_confirmed(prior, "heads")
        )
        self.commit_repository_state(
            repo,
            {
                "heads": stored,
                "repo_updated_on": self.cursor_value(prior, repo_updated_on, complete),
            },
        )

    def _changes(
        self,
        repo: RepositoryRef,
        branch: BranchRef,
        include: str,
        exclude: str | None,
        action: str,
        unresolved: set[str],
    ) -> Iterable[Mapping[str, Any]]:
        # Recovery below has to replace the whole range, so nothing may have
        # been emitted when it runs — but a first read of a branch spans its
        # entire history and several repositories are in flight at once. Hold
        # only the head of the range: the API rejects a stale exclude on the
        # first request, so a recoverable failure lands inside this window,
        # while a longer range spills into a plain stream.
        prefetched: list[Mapping[str, Any]] = []
        commits: Iterator[Mapping[str, Any]] = iter(())
        try:
            commits = iter(self._client.commits_between(repo, [include], [exclude] if exclude else []))
            for commit in commits:
                prefetched.append(commit)
                if len(prefetched) >= RANGE_PREFETCH:
                    break
        except BitbucketApiError as exc:
            if exc.status_code == 404 and include in exc.missing_shas:
                # The head this range starts from is gone: nothing is reachable
                # from it, and no other branch of the repository is affected.
                unresolved.add(branch.name)
                return
            if exc.status_code != 404 or not exclude:
                raise
            if action == "added":
                commits = self._client.commits_between(repo, [include], [])
                yield from self._reachability_records(repo, branch, include, "reset", commits)
                return
            entity_key = unique_key(
                self._tenant_id,
                self._source_id,
                *repo_scope(repo),
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
        yield from self._reachability_records(repo, branch, include, action, chain(prefetched, commits))

    def _reachability_records(self, repo, branch, head, action, commits):
        for commit in commits:
            committed_at = commit.get("date")
            if self.before_start_date(committed_at):
                continue
            sha = str(commit.get("hash") or "")
            if not sha:
                continue
            entity_key = unique_key(
                self._tenant_id,
                self._source_id,
                *repo_scope(repo),
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
