from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from airbyte_cdk.models import SyncMode

from source_bitbucket_cloud.streams.base import BitbucketStream, schema, unique_key


class RepositoriesStream(BitbucketStream):
    name = "repositories"

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any]]:
        del sync_mode, cursor_field, stream_state
        bucket_id = int((stream_slice or {}).get("bucket_id", 0))
        repositories = self.repositories_for_slice(stream_slice)
        generation = self.generation("repositories", bucket_id)
        entity_keys: set[str] = set()
        for repo in repositories:
            raw = repo.raw
            owner = raw.get("owner") or {}
            project = raw.get("project") or {}
            parent = raw.get("parent") or {}
            entity_key = unique_key(self._tenant_id, self._source_id, repo.uuid)
            entity_keys.add(entity_key)
            yield self.item(
                entity_key=entity_key,
                generation_id=generation,
                bucket_id=bucket_id,
                repository_uuid=repo.uuid,
                workspace_uuid=repo.workspace_uuid,
                workspace=repo.workspace,
                slug=repo.slug,
                name=raw.get("name"),
                full_name=raw.get("full_name"),
                uuid=repo.uuid,
                is_private=raw.get("is_private"),
                description=raw.get("description"),
                language=raw.get("language"),
                size=raw.get("size"),
                created_on=raw.get("created_on"),
                updated_on=raw.get("updated_on"),
                has_issues=raw.get("has_issues"),
                has_wiki=raw.get("has_wiki"),
                mainbranch_name=repo.mainbranch_name,
                scm=raw.get("scm"),
                fork_policy=raw.get("fork_policy"),
                website=raw.get("website"),
                owner_uuid=owner.get("uuid"),
                owner_account_id=owner.get("account_id"),
                owner_display_name=owner.get("display_name"),
                owner_nickname=owner.get("nickname"),
                workspace_slug=repo.workspace,
                parent_uuid=parent.get("uuid"),
                parent_full_name=parent.get("full_name"),
                project_key=project.get("key"),
                project_name=project.get("name"),
                project_uuid=project.get("uuid"),
                links=raw.get("links"),
            )
        yield self.complete(
            scope_parts=["repositories", bucket_id],
            generation_id=generation,
            item_count=len(entity_keys),
            bucket_id=bucket_id,
        )
        self.finish_bucket(bucket_id, repositories)

    def get_json_schema(self) -> Mapping[str, Any]:
        nullable_string = {"type": ["null", "string"]}
        return schema(
            {
                "workspace": {"type": ["null", "string"]},
                "slug": nullable_string,
                "name": nullable_string,
                "full_name": nullable_string,
                "uuid": nullable_string,
                "is_private": {"type": ["null", "boolean"]},
                "description": nullable_string,
                "language": nullable_string,
                "size": {"type": ["null", "integer"]},
                "created_on": nullable_string,
                "updated_on": nullable_string,
                "has_issues": {"type": ["null", "boolean"]},
                "has_wiki": {"type": ["null", "boolean"]},
                "mainbranch_name": nullable_string,
                "scm": nullable_string,
                "fork_policy": nullable_string,
                "website": nullable_string,
                "owner_uuid": nullable_string,
                "owner_account_id": nullable_string,
                "owner_display_name": nullable_string,
                "owner_nickname": nullable_string,
                "workspace_slug": nullable_string,
                "parent_uuid": nullable_string,
                "parent_full_name": nullable_string,
                "project_key": nullable_string,
                "project_name": nullable_string,
                "project_uuid": nullable_string,
                "links": {"type": ["null", "object"]},
            }
        )
