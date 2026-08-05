from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from source_bitbucket_cloud.streams.base import BitbucketIncrementalStream, repo_scope, schema, unique_key


class BranchesStream(BitbucketIncrementalStream):
    """Current branches per repository, as per-repository snapshots.

    The generation is scoped to one repository, not to a bucket: a repository
    that is denied (403) or fails simply produces no marker this sync, so dbt
    keeps its previous branch generation, while every other repository updates
    independently. A bucket-scoped generation would freeze branch updates for a
    whole bucket over one denied repository — and in a workspace where
    unreadable repositories are common, every bucket would freeze.

    Incremental only in the cheapest sense: the per-repository state holds the
    repository's updated_on from the workspace listing, and a repository that
    has not been pushed to since the last pass is skipped without a request —
    its previous generation simply stays the newest complete one.

    start_date does not apply here. Branches are current state, not dated
    history, so a repository last pushed before the window still has branches
    worth reporting; the idle gate already keeps it at one listing ever.

    Trade-off: a repository deleted from the workspace stops producing
    generations, so its last branch snapshot lingers in silver. That is bounded
    (the repository is gone) and preferable to fleet-wide starvation.
    """

    name = "branches"
    cursor_field = "updated_on"

    def repository_records(self, repo, bucket_id: int) -> Iterable[Mapping[str, Any]]:
        prior = self.repository_state(repo)
        repo_updated_on = str(repo.raw.get("updated_on") or "")
        if repo_updated_on and prior.get("repo_updated_on") == repo_updated_on:
            return
        branches = self._catalog.branches(repo)
        if not branches and prior.get("branch_count") != 0:
            # A snapshot replaces the previous one, so publishing an empty one
            # deletes every branch this repository had. One empty answer is
            # never enough to do that — including the first one seen, since
            # state written before this rule existed carries no count. Record
            # the observation and hold the cursor; a second consecutive empty
            # listing is the repository, not the API, and publishes normally.
            self.commit_repository_state(
                repo, {"repo_updated_on": str(prior.get("repo_updated_on") or ""), "branch_count": 0}
            )
            return

        generation = self.generation("branches", *repo_scope(repo))
        entity_keys: set[str] = set()
        for branch in branches:
            entity_key = unique_key(self._tenant_id, self._source_id, *repo_scope(repo), branch.name)
            entity_keys.add(entity_key)
            yield self.item(
                entity_key=entity_key,
                generation_id=generation,
                bucket_id=bucket_id,
                repository_uuid=repo.uuid,
                workspace_uuid=repo.workspace_uuid,
                workspace=repo.workspace,
                repo_slug=repo.slug,
                name=branch.name,
                target_hash=branch.head_sha,
                target_date=branch.target_date,
                mainbranch_name=repo.mainbranch_name,
                default_branch_name=repo.mainbranch_name,
                is_default=branch.is_default,
                updated_on=repo.raw.get("updated_on"),
            )
        yield self.complete(
            scope_parts=["branches", *repo_scope(repo)],
            generation_id=generation,
            item_count=len(entity_keys),
            bucket_id=bucket_id,
            repository_uuid=repo.uuid,
            workspace_uuid=repo.workspace_uuid,
            workspace=repo.workspace,
            repo_slug=repo.slug,
        )
        self.commit_repository_state(
            repo, {"repo_updated_on": repo_updated_on, "branch_count": len(entity_keys)}
        )

    def get_json_schema(self) -> Mapping[str, Any]:
        nullable_string = {"type": ["null", "string"]}
        return schema(
            {
                "workspace": nullable_string,
                "repo_slug": nullable_string,
                "name": nullable_string,
                "target_hash": nullable_string,
                "target_date": nullable_string,
                "mainbranch_name": nullable_string,
                "default_branch_name": nullable_string,
                "is_default": {"type": ["null", "boolean"]},
                "updated_on": nullable_string,
            }
        )
