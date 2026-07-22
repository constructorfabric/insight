from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from source_bitbucket_cloud.streams.base import AUTHOR_RE, BitbucketIncrementalStream, schema, truncate, unique_key
from source_bitbucket_cloud.streams.git_ranges import CommitRangeMixin


class CommitsStream(CommitRangeMixin, BitbucketIncrementalStream):
    name = "commits"
    cursor_field = "date"

    def repository_records(self, repo, bucket_id: int) -> Iterable[Mapping[str, Any]]:
        del bucket_id
        prior = self.repository_state(repo)
        _, current_heads = self.branch_snapshot(repo)
        current_head_shas = sorted(set(current_heads.values()))
        previous_head_shas = prior.get("head_shas") or []
        if current_head_shas != previous_head_shas:
            for commit in self.new_commits(repo, current_head_shas, previous_head_shas):
                record = self._record(repo, commit)
                if self._start_date and record.get("date") and str(record["date"])[:10] < self._start_date:
                    continue
                yield record
        self.commit_repository_state(repo, {"head_shas": current_head_shas})

    def _record(self, repo, commit: Mapping[str, Any]) -> Mapping[str, Any]:
        sha = str(commit.get("hash") or "")
        author = self._identity(commit.get("author") or {})
        committer = self._identity(commit.get("committer") or {})
        entity_key = unique_key(self._tenant_id, self._source_id, repo.uuid, sha)
        return self.item(
            entity_key=entity_key,
            repository_uuid=repo.uuid,
            workspace_uuid=repo.workspace_uuid,
            hash=sha,
            message=truncate(commit.get("message")),
            date=commit.get("date"),
            author_raw=author["raw"],
            author_name=author["name"],
            author_email=author["email"],
            author_display_name=author["display_name"],
            author_uuid=author["uuid"],
            author_account_id=author["account_id"],
            committer_raw=committer["raw"],
            committer_name=committer["name"],
            committer_email=committer["email"],
            committer_display_name=committer["display_name"],
            committer_uuid=committer["uuid"],
            committer_account_id=committer["account_id"],
            parent_hashes=[parent.get("hash") for parent in commit.get("parents") or [] if parent.get("hash")],
            workspace=repo.workspace,
            repo_slug=repo.slug,
            branch_name=None,
            head_sha=None,
        )

    def _identity(self, value: Mapping[str, Any]) -> Mapping[str, str | None]:
        raw = str(value.get("raw") or "")
        name = raw or None
        email = None
        match = AUTHOR_RE.match(raw)
        if match:
            name = match.group(1).strip() or None
            email = match.group(2).strip() or None
        user = value.get("user") or {}
        return {
            "raw": raw or None,
            "name": name,
            "email": email,
            "display_name": user.get("display_name"),
            "uuid": user.get("uuid"),
            "account_id": user.get("account_id"),
        }

    def get_json_schema(self) -> Mapping[str, Any]:
        nullable_string = {"type": ["null", "string"]}
        return schema(
            {
                "hash": nullable_string,
                "message": nullable_string,
                "date": nullable_string,
                "author_raw": nullable_string,
                "author_name": nullable_string,
                "author_email": nullable_string,
                "author_display_name": nullable_string,
                "author_uuid": nullable_string,
                "author_account_id": nullable_string,
                "committer_raw": nullable_string,
                "committer_name": nullable_string,
                "committer_email": nullable_string,
                "committer_display_name": nullable_string,
                "committer_uuid": nullable_string,
                "committer_account_id": nullable_string,
                "parent_hashes": {"type": ["null", "array"]},
                "workspace": nullable_string,
                "repo_slug": nullable_string,
                "branch_name": nullable_string,
                "head_sha": nullable_string,
            }
        )
