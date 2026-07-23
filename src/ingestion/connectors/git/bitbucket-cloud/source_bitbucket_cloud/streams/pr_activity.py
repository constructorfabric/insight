from __future__ import annotations

import hashlib
import json
from collections.abc import Iterable, Mapping
from typing import Any

from source_bitbucket_cloud.streams.base import schema, unique_key
from source_bitbucket_cloud.streams.pr_base import PullRequestStateStream


class PRActivityStream(PullRequestStateStream):
    name = "pull_request_activity"

    def pull_request_records(self, repo, pr: Mapping[str, Any]) -> Iterable[Mapping[str, Any]]:
        pr_id = pr.get("id")
        updated_on = pr.get("updated_on")
        revision = self.pull_request_revision(pr)
        generation = self.generation(repo.uuid, pr_id, "activity")
        entity_keys: set[str] = set()
        path = self._client.repo_path(repo, f"pullrequests/{pr_id}/activity")
        present, entries = self._client.paginate_optional(path, params={"pagelen": "100"})
        for entry in entries:
            update = entry.get("update") or {}
            approval = entry.get("approval") or {}
            comment = entry.get("comment") or {}
            if update:
                event_type = "update"
            elif approval:
                event_type = "approval"
            elif comment:
                event_type = "comment"
            else:
                event_type = "unknown"
            actor = entry.get("user") or update.get("author") or approval.get("user") or comment.get("user") or {}
            activity_date = (
                update.get("date")
                or approval.get("date")
                or comment.get("created_on")
                or entry.get("created_on")
                or entry.get("date")
            )
            raw_identity = json.dumps(entry, sort_keys=True, separators=(",", ":"), default=str).encode("utf-8")
            activity_id = entry.get("id") or hashlib.sha256(raw_identity).hexdigest()
            entity_key = unique_key(self._tenant_id, self._source_id, repo.uuid, pr_id, activity_id, event_type)
            entity_keys.add(entity_key)
            yield self.item(
                entity_key=entity_key,
                generation_id=generation,
                repository_uuid=repo.uuid,
                workspace_uuid=repo.workspace_uuid,
                pr_id=pr_id,
                event_type=event_type,
                activity_date=activity_date,
                update_state=update.get("state"),
                actor_display_name=actor.get("display_name"),
                actor_uuid=actor.get("uuid"),
                actor_account_id=actor.get("account_id"),
                pull_request_updated_on=updated_on,
                **revision,
                workspace=repo.workspace,
                repo_slug=repo.slug,
            )
        yield self.complete(
            scope_parts=[repo.uuid, pr_id, "activity"],
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
                "event_type": nullable_string,
                "activity_date": nullable_string,
                "update_state": nullable_string,
                "actor_display_name": nullable_string,
                "actor_uuid": nullable_string,
                "actor_account_id": nullable_string,
                "pull_request_updated_on": nullable_string,
                "pull_request_source_commit_hash": nullable_string,
                "pull_request_destination_commit_hash": nullable_string,
                "workspace": nullable_string,
                "repo_slug": nullable_string,
            }
        )
