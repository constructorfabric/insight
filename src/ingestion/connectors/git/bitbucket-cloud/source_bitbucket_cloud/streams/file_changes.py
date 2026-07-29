from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from source_bitbucket_cloud.streams.base import BitbucketIncrementalStream, repo_scope, schema, unique_key
from source_bitbucket_cloud.streams.git_ranges import CommitRangeMixin


class FileChangesStream(CommitRangeMixin, BitbucketIncrementalStream):
    name = "file_changes"
    cursor_field = "committed_date"

    def repository_records(self, repo, bucket_id: int) -> Iterable[Mapping[str, Any]]:
        del bucket_id
        prior = self.repository_state(repo)
        repo_updated_on = str(repo.raw.get("updated_on") or "")
        if repo_updated_on and prior.get("repo_updated_on") == repo_updated_on:
            # The repository has not been pushed to since the last successful
            # pass (updated_on comes free with the workspace listing), so the
            # branch heads cannot have moved: skip the branch listing and the
            # range fetch entirely. This is what keeps the per-repository
            # request budget at zero for the idle majority of a large fleet.
            return
        _, current_heads = self.branch_snapshot(repo)
        current_head_shas = sorted(set(current_heads.values()))
        previous_head_shas = prior.get("head_shas") or []
        if current_head_shas != previous_head_shas:
            for commit in self.new_commits(repo, current_head_shas, previous_head_shas):
                committed_date = commit.get("date")
                if self._start_date and committed_date and str(committed_date)[:10] < self._start_date:
                    continue
                yield from self._diffstat(repo, str(commit.get("hash") or ""), committed_date)
        self.commit_repository_state(repo, {"head_shas": current_head_shas, "repo_updated_on": repo_updated_on})

    def _diffstat(self, repo, sha: str, committed_date: Any) -> Iterable[Mapping[str, Any]]:
        if not sha:
            return
        generation = self.generation(repo.uuid, sha)
        entity_keys: set[str] = set()
        # A commit's diffstat can be permanently gone (orphaned merge parents,
        # rewritten history) — the pre-rewrite connector tolerated exactly this
        # ("commit diffstat gone", ignore_404). Raising here would fail the
        # repository on every sync forever. The marker records the denial so the
        # completeness gate keeps whatever was known before instead of treating
        # the empty read as "this commit changed nothing".
        present, entries = self._client.paginate_optional(
            self._client.repo_path(repo, f"diffstat/{sha}"), params={"pagelen": "100"}
        )
        for entry in entries:
            new_file = entry.get("new") or {}
            old_file = entry.get("old") or {}
            filename = new_file.get("path") or old_file.get("path")
            if not filename:
                continue
            status = entry.get("status")
            entity_key = unique_key(self._tenant_id, self._source_id, *repo_scope(repo), sha, filename)
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
            available=present,
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
