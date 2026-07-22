from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from source_bitbucket_cloud.streams.base import schema, truncate, unique_key
from source_bitbucket_cloud.streams.pr_base import PullRequestStateStream


class PRCommentsStream(PullRequestStateStream):
    name = "pull_request_comments"

    def pull_request_records(self, repo, pr: Mapping[str, Any]) -> Iterable[Mapping[str, Any]]:
        pr_id = pr.get("id")
        updated_on = pr.get("updated_on")
        revision = self.pull_request_revision(pr)
        generation = self.generation(repo.uuid, pr_id, "comments")
        entity_keys: set[str] = set()
        path = self._client.repo_path(repo, f"pullrequests/{pr_id}/comments")
        present, comments = self._client.paginate_optional(path, params={"pagelen": "100"})
        for comment in comments:
            comment_id = comment.get("id")
            if comment_id is None:
                continue
            user = comment.get("user") or {}
            inline = comment.get("inline") or {}
            parent = comment.get("parent") or {}
            entity_key = unique_key(self._tenant_id, self._source_id, repo.uuid, pr_id, comment_id)
            entity_keys.add(entity_key)
            yield self.item(
                entity_key=entity_key,
                generation_id=generation,
                repository_uuid=repo.uuid,
                workspace_uuid=repo.workspace_uuid,
                comment_id=comment_id,
                pr_id=pr_id,
                body=truncate((comment.get("content") or {}).get("raw")),
                created_on=comment.get("created_on"),
                updated_on=comment.get("updated_on"),
                author_display_name=user.get("display_name"),
                author_uuid=user.get("uuid"),
                author_account_id=user.get("account_id"),
                is_inline=bool(inline),
                inline_path=inline.get("path"),
                inline_from=inline.get("from"),
                inline_to=inline.get("to"),
                parent_comment_id=parent.get("id"),
                is_deleted=comment.get("deleted"),
                pull_request_updated_on=updated_on,
                **revision,
                workspace=repo.workspace,
                repo_slug=repo.slug,
            )
        yield self.complete(
            scope_parts=[repo.uuid, pr_id, "comments"],
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
                "comment_id": {"type": ["null", "integer"]},
                "pr_id": {"type": ["null", "integer"]},
                "body": nullable_string,
                "created_on": nullable_string,
                "updated_on": nullable_string,
                "author_display_name": nullable_string,
                "author_uuid": nullable_string,
                "author_account_id": nullable_string,
                "is_inline": {"type": ["null", "boolean"]},
                "inline_path": nullable_string,
                "inline_from": {"type": ["null", "integer"]},
                "inline_to": {"type": ["null", "integer"]},
                "parent_comment_id": {"type": ["null", "integer"]},
                "is_deleted": {"type": ["null", "boolean"]},
                "pull_request_updated_on": nullable_string,
                "pull_request_source_commit_hash": nullable_string,
                "pull_request_destination_commit_hash": nullable_string,
                "workspace": nullable_string,
                "repo_slug": nullable_string,
            }
        )
