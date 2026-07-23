from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from source_bitbucket_cloud.streams.base import BitbucketIncrementalStream, schema, unique_key
from source_bitbucket_cloud.streams.git_ranges import CommitRangeMixin


class FileChangesStream(CommitRangeMixin, BitbucketIncrementalStream):
    name = "file_changes"
    cursor_field = "committed_date"

    def repository_records(self, repo, bucket_id: int) -> Iterable[Mapping[str, Any]]:
        del bucket_id
        prior = self.repository_state(repo)
        _, current_heads = self.branch_snapshot(repo)
        current_head_shas = sorted(set(current_heads.values()))
        previous_head_shas = prior.get("head_shas") or []
        if current_head_shas != previous_head_shas:
            for commit in self.new_commits(repo, current_head_shas, previous_head_shas):
                committed_date = commit.get("date")
                if self._start_date and committed_date and str(committed_date)[:10] < self._start_date:
                    continue
                yield from self._diffstat(repo, str(commit.get("hash") or ""), committed_date)
        self.commit_repository_state(repo, {"head_shas": current_head_shas})

    def _diffstat(self, repo, sha: str, committed_date: Any) -> Iterable[Mapping[str, Any]]:
        if not sha:
            return
        generation = self.generation(repo.uuid, sha)
        entity_keys: set[str] = set()
        for entry in self._client.paginate(self._client.repo_path(repo, f"diffstat/{sha}"), params={"pagelen": "100"}):
            new_file = entry.get("new") or {}
            old_file = entry.get("old") or {}
            filename = new_file.get("path") or old_file.get("path")
            if not filename:
                continue
            status = entry.get("status")
            entity_key = unique_key(self._tenant_id, self._source_id, repo.uuid, sha, filename)
            entity_keys.add(entity_key)
            yield self.item(
                entity_key=entity_key,
                generation_id=generation,
                repository_uuid=repo.uuid,
                workspace_uuid=repo.workspace_uuid,
                source_type="commit",
                sha=sha,
                is_snapshot_marker=False,
                marker_type=None,
                filename=filename,
                status=status,
                additions=entry.get("lines_added"),
                deletions=entry.get("lines_removed"),
                previous_filename=old_file.get("path") if status == "renamed" else None,
                committed_date=committed_date,
                workspace=repo.workspace,
                repo_slug=repo.slug,
            )
        yield self.complete(
            scope_parts=[repo.uuid, sha, "diffstat"],
            generation_id=generation,
            item_count=len(entity_keys),
            repository_uuid=repo.uuid,
            workspace_uuid=repo.workspace_uuid,
            source_type="commit",
            sha=sha,
            is_snapshot_marker=True,
            marker_type="commit_snapshot_complete",
            committed_date=committed_date,
            workspace=repo.workspace,
            repo_slug=repo.slug,
        )

    def get_json_schema(self) -> Mapping[str, Any]:
        nullable_string = {"type": ["null", "string"]}
        return schema(
            {
                "source_type": nullable_string,
                "sha": nullable_string,
                "is_snapshot_marker": {"type": ["null", "boolean"]},
                "marker_type": nullable_string,
                "filename": nullable_string,
                "status": nullable_string,
                "additions": {"type": ["null", "integer"]},
                "deletions": {"type": ["null", "integer"]},
                "previous_filename": nullable_string,
                "committed_date": nullable_string,
                "workspace": nullable_string,
                "repo_slug": nullable_string,
            }
        )
