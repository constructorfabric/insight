from __future__ import annotations

from collections.abc import Mapping
from datetime import datetime, timedelta
from typing import Any

from source_bitbucket_cloud.client import RepositoryRef
from source_bitbucket_cloud.streams.base import BitbucketIncrementalStream

PR_STATES = ("OPEN", "MERGED", "DECLINED", "SUPERSEDED")
TERMINAL_PR_STATES = ("MERGED", "DECLINED", "SUPERSEDED")
RECONCILE_LIMIT = 100
OVERLAP_MINUTES = 5


class PullRequestStateStream(BitbucketIncrementalStream):
    cursor_field = "updated_on"

    def repository_records(self, repo, bucket_id):
        del bucket_id
        selected, new_state = self.selected_pull_requests(repo, self.repository_state(repo))
        for pr in selected:
            yield from self.pull_request_records(repo, pr)
        self.commit_repository_state(repo, new_state)

    def pull_request_records(self, repo, pr: Mapping[str, Any]):
        raise NotImplementedError

    def selected_pull_requests(
        self, repo: RepositoryRef, prior: Mapping[str, Any]
    ) -> tuple[list[Mapping[str, Any]], Mapping[str, Any]]:
        selected: dict[int, Mapping[str, Any]] = {}
        watermark = str(prior.get("updated_on") or "")
        floor = self._overlap(watermark) or self._start_date
        params: list[tuple[str, Any]] = [("pagelen", "50"), ("sort", "updated_on"), ("fields", self._pr_fields())]
        params.extend(("state", state) for state in PR_STATES)
        if floor:
            params.append(("q", f'updated_on>="{floor}"'))
        for pr in self._client.paginate(self._client.repo_path(repo, "pullrequests"), params=params):
            pr_id = pr.get("id")
            if pr_id is not None:
                selected[int(pr_id)] = pr

        if watermark:
            open_params: list[tuple[str, Any]] = [
                ("pagelen", "50"),
                ("state", "OPEN"),
                ("sort", "id"),
                ("fields", self._pr_fields()),
            ]
            for pr in self._client.paginate(self._client.repo_path(repo, "pullrequests"), params=open_params):
                pr_id = pr.get("id")
                if pr_id is not None:
                    selected[int(pr_id)] = pr

        reconcile_after = int(prior.get("reconcile_after_id") or 0)
        next_reconcile = reconcile_after
        if watermark:
            # The /pullrequests endpoint caps pagelen at 50 and rejects larger
            # values with HTTP 400 "Invalid pagelen" (Atlassian BCLOUD-13229),
            # same as the listing query above. RECONCILE_LIMIT is still honoured
            # across pages — paginate() accumulates until the caller stops.
            reconcile_params: list[tuple[str, Any]] = [
                ("pagelen", "50"),
                ("sort", "id"),
                ("fields", self._pr_fields()),
            ]
            reconcile_params.extend(("state", state) for state in TERMINAL_PR_STATES)
            if reconcile_after:
                reconcile_params.append(("q", f"id>{reconcile_after}"))
            reconciled = []
            for pr in self._client.paginate(self._client.repo_path(repo, "pullrequests"), params=reconcile_params):
                reconciled.append(pr)
                if len(reconciled) > RECONCILE_LIMIT:
                    break
            for pr in reconciled[:RECONCILE_LIMIT]:
                pr_id = pr.get("id")
                if pr_id is not None:
                    selected[int(pr_id)] = pr
                    next_reconcile = max(next_reconcile, int(pr_id))
            if len(reconciled) <= RECONCILE_LIMIT:
                next_reconcile = 0

        new_state = {
            "updated_on": max([watermark, *(str(pr.get("updated_on") or "") for pr in selected.values())]),
            "reconcile_after_id": next_reconcile,
        }
        return list(selected.values()), new_state

    def _overlap(self, value: str) -> str | None:
        if not value:
            return None
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        return (parsed - timedelta(minutes=OVERLAP_MINUTES)).isoformat()

    def _pr_fields(self) -> str:
        return (
            "values.id,values.title,values.description,values.state,values.created_on,"
            "values.updated_on,values.author,values.closed_by,values.source.branch.name,"
            "values.source.commit.hash,values.destination.branch.name,"
            "values.destination.commit.hash,values.merge_commit.hash,values.task_count,"
            "values.draft,values.queued,values.close_source_branch,values.reason,"
            "values.reviewers,values.participants,values.comment_count,next"
        )

    def pull_request_revision(self, pr: Mapping[str, Any]) -> Mapping[str, str | None]:
        source = pr.get("source") or {}
        destination = pr.get("destination") or {}
        return {
            "pull_request_source_commit_hash": (source.get("commit") or {}).get("hash"),
            "pull_request_destination_commit_hash": (destination.get("commit") or {}).get("hash"),
        }
