from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from source_bitbucket_cloud.streams.base import AUTHOR_RE, BitbucketIncrementalStream, repo_scope, schema, truncate, unique_key
from source_bitbucket_cloud.streams.git_ranges import CommitRangeMixin


class CommitsStream(CommitRangeMixin, BitbucketIncrementalStream):
    name = "commits"
    cursor_field = "date"

    def repository_records(self, repo, bucket_id: int) -> Iterable[Mapping[str, Any]]:
        del bucket_id
        if self.out_of_window(repo):
            return
        prior = self.repository_state(repo)
        repo_updated_on = str(repo.raw.get("updated_on") or "")
        if repo_updated_on and prior.get("repo_updated_on") == repo_updated_on:
            # The repository has not been pushed to since the last successful
            # pass (updated_on comes free with the workspace listing), so the
            # branch heads cannot have moved: skip the branch listing and the
            # range fetch entirely. This is what keeps the per-repository
            # request budget at zero for the idle majority of a large fleet.
            return
        branches, current_heads = self.branch_snapshot(repo)
        current_head_shas = sorted(set(current_heads.values()))
        previous_head_shas = prior.get("head_shas") or []
        unresolved: set[str] = set()
        if current_head_shas != previous_head_shas:
            includes = current_head_shas if previous_head_shas else self.cold_includes(branches)
            for commit in self.new_commits(repo, includes, previous_head_shas, unresolved):
                record = self._record(repo, commit)
                if self.before_start_date(record.get("date")):
                    continue
                yield record

        stored = [sha for sha in self.retained_heads(current_head_shas, previous_head_shas) if sha not in unresolved]
        complete = self.complete_read(
            current_head_shas, unresolved, empty_confirmed=self.empty_listing_confirmed(prior, "head_shas")
        )
        self.commit_repository_state(
            repo,
            {
                "head_shas": stored,
                "repo_updated_on": self.cursor_value(prior, repo_updated_on, complete),
            },
        )

    def _record(self, repo, commit: Mapping[str, Any]) -> Mapping[str, Any]:
        sha = str(commit.get("hash") or "")
        author = self._identity(commit.get("author") or {})
        committer = self._identity(commit.get("committer") or {})
        entity_key = unique_key(self._tenant_id, self._source_id, *repo_scope(repo), sha)
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
