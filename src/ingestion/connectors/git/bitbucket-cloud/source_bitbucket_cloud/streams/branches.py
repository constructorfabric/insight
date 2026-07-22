from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from airbyte_cdk.models import SyncMode

from source_bitbucket_cloud.streams.base import BitbucketStream, schema, unique_key


class BranchesStream(BitbucketStream):
    name = "branches"

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: list[str] | None = None,
        stream_slice: Mapping[str, Any] | None = None,
        stream_state: Mapping[str, Any] | None = None,
    ) -> Iterable[Mapping[str, Any]]:
        del sync_mode, cursor_field, stream_state
        bucket_id, repositories = self.bucket(stream_slice)
        generation = self.generation("branches", bucket_id)
        entity_keys: set[str] = set()
        failures_before = len(self._failed_repositories)
        for repo in repositories:
            try:
                for branch in self._catalog.branches(repo):
                    entity_key = unique_key(self._tenant_id, self._source_id, repo.uuid, branch.name)
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
            except Exception:
                self.record_failure(repo)
        yield self.complete(
            scope_parts=["branches", bucket_id],
            generation_id=generation,
            item_count=len(entity_keys),
            bucket_id=bucket_id,
            available=len(self._failed_repositories) == failures_before,
        )
        self.finish_bucket(bucket_id, repositories)

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
