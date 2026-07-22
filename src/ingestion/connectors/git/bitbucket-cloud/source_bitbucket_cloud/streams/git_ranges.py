from __future__ import annotations

from collections.abc import Iterable, Mapping, Sequence

from source_bitbucket_cloud.client import BitbucketApiError, BranchRef, RepositoryCatalog, RepositoryRef


class CommitRangeMixin:
    _client: object
    _catalog: RepositoryCatalog

    def branch_snapshot(self, repo: RepositoryRef) -> tuple[list[BranchRef], dict[str, str]]:
        branches = self._catalog.branches(repo)
        return branches, {branch.name: branch.head_sha for branch in branches}

    def new_commits(
        self,
        repo: RepositoryRef,
        current_heads: Sequence[str],
        previous_heads: Sequence[str],
    ) -> Iterable[Mapping[str, object]]:
        try:
            yield from self._client.commits_between(repo, current_heads, previous_heads)
        except BitbucketApiError as exc:
            if exc.status_code != 404 or not previous_heads:
                raise
            yield from self._client.commits_between(repo, current_heads, [])
