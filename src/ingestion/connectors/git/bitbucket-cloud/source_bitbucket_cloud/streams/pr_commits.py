from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from source_bitbucket_cloud.streams.base import schema, unique_key
from source_bitbucket_cloud.streams.pr_base import PullRequestStateStream


class PRCommitsStream(PullRequestStateStream):
    name = "pull_request_commits"

    def pull_request_records(self, repo, pr: Mapping[str, Any]) -> Iterable[Mapping[str, Any]]:
        pr_id = pr.get("id")
        updated_on = pr.get("updated_on")
        revision = self.pull_request_revision(pr)
        generation = self.generation(repo.uuid, pr_id, "commits")
        entity_keys: set[str] = set()
        path = self._client.repo_path(repo, f"pullrequests/{pr_id}/commits")
        present, commits = self._client.paginate_optional(path, params={"pagelen": "100"})
        for commit_order, commit in enumerate(commits):
            sha = str(commit.get("hash") or "")
            if not sha:
                continue
            user = (commit.get("author") or {}).get("user") or {}
            entity_key = unique_key(self._tenant_id, self._source_id, repo.uuid, pr_id, sha)
            entity_keys.add(entity_key)
            yield self.item(
                entity_key=entity_key,
                generation_id=generation,
                repository_uuid=repo.uuid,
                workspace_uuid=repo.workspace_uuid,
                pr_id=pr_id,
                hash=sha,
                commit_order=commit_order,
                author_uuid=user.get("uuid"),
                author_account_id=user.get("account_id"),
                pull_request_updated_on=updated_on,
                **revision,
                workspace=repo.workspace,
                repo_slug=repo.slug,
            )
        yield self.complete(
            scope_parts=[repo.uuid, pr_id, "commits"],
            generation_id=generation,
            item_count=len(entity_keys),
            available=present,
            repository_uuid=repo.uuid,
            workspace_uuid=repo.workspace_uuid,
            pr_id=pr_id,
            pull_request_updated_on=updated_on,
            **revision,
            workspace=repo.workspace,
            repo_slug=repo.slug,
        )

    def get_json_schema(self) -> Mapping[str, Any]:
        nullable_string = {"type": ["null", "string"]}
        return schema(
            {
                "pr_id": {"type": ["null", "integer"]},
                "hash": nullable_string,
                "commit_order": {"type": ["null", "integer"]},
                "author_uuid": nullable_string,
                "author_account_id": nullable_string,
                "pull_request_updated_on": nullable_string,
                "pull_request_source_commit_hash": nullable_string,
                "pull_request_destination_commit_hash": nullable_string,
                "workspace": nullable_string,
                "repo_slug": nullable_string,
            }
        )
