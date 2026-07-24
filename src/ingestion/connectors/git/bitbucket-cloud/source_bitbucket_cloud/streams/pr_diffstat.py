from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from source_bitbucket_cloud.streams.base import schema, unique_key
from source_bitbucket_cloud.streams.pr_base import PullRequestStateStream


class PRDiffstatStream(PullRequestStateStream):
    name = "pull_request_diffstat"

    def pull_request_records(self, repo, pr: Mapping[str, Any]) -> Iterable[Mapping[str, Any]]:
        pr_id = pr.get("id")
        updated_on = pr.get("updated_on")
        revision = self.pull_request_revision(pr)
        generation = self.generation(repo.uuid, pr_id, "diffstat")
        path = self._client.repo_path(repo, f"pullrequests/{pr_id}/diffstat")
        entity_keys: set[str] = set()
        present, entries = self._client.paginate_optional(path, params={"pagelen": "100"})
        for entry in entries:
            new_file = entry.get("new") or {}
            old_file = entry.get("old") or {}
            file_path = new_file.get("path") or old_file.get("path")
            if not file_path:
                continue
            entity_key = unique_key(self._tenant_id, self._source_id, repo.uuid, pr_id, file_path)
            entity_keys.add(entity_key)
            yield self.item(
                entity_key=entity_key,
                generation_id=generation,
                repository_uuid=repo.uuid,
                workspace_uuid=repo.workspace_uuid,
                pr_id=pr_id,
                is_snapshot_marker=False,
                status=entry.get("status"),
                old_path=old_file.get("path"),
                new_path=new_file.get("path"),
                lines_added=entry.get("lines_added"),
                lines_removed=entry.get("lines_removed"),
                pull_request_updated_on=updated_on,
                **revision,
                workspace=repo.workspace,
                repo_slug=repo.slug,
            )
        yield self.complete(
            scope_parts=[repo.uuid, pr_id, "diffstat"],
            generation_id=generation,
            item_count=len(entity_keys),
            available=present,
            repository_uuid=repo.uuid,
            workspace_uuid=repo.workspace_uuid,
            pr_id=pr_id,
            is_snapshot_marker=True,
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
                "is_snapshot_marker": {"type": ["null", "boolean"]},
                "status": nullable_string,
                "old_path": nullable_string,
                "new_path": nullable_string,
                "lines_added": {"type": ["null", "integer"]},
                "lines_removed": {"type": ["null", "integer"]},
                "pull_request_updated_on": nullable_string,
                "pull_request_source_commit_hash": nullable_string,
                "pull_request_destination_commit_hash": nullable_string,
                "workspace": nullable_string,
                "repo_slug": nullable_string,
            }
        )
