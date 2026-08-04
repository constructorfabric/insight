from __future__ import annotations

from collections.abc import Mapping
from datetime import datetime, timedelta
from typing import Any

from source_bitbucket_cloud.streams.base import (
    BitbucketIncrementalStream,
    BitbucketStream,
    repo_scope,
    repo_state_key,
    schema,
    unique_key,
)
from source_bitbucket_cloud.streams.pr_base import PullRequestStateStream


class RepositorySnapshotStream(BitbucketStream):
    resource = ""

    def repository_records(self, repo, bucket_id: int):
        if not self.enabled(repo):
            return
        generation = self.generation(self.name, repo.uuid)
        entity_keys: set[str] = set()
        present, records = self._client.paginate_optional(
            self._client.repo_path(repo, self.resource), params={"pagelen": "100"}
        )
        for record in records:
            identity = record.get("uuid") or record.get("id") or record.get("name")
            if identity is None or not self.include(record):
                continue
            entity_key = unique_key(self._tenant_id, self._source_id, *repo_scope(repo), identity)
            entity_keys.add(entity_key)
            projected = dict(record)
            projected.update(self.project(record))
            projected.update(
                {
                    "bucket_id": bucket_id,
                    "repository_uuid": repo.uuid,
                    "workspace_uuid": repo.workspace_uuid,
                    "workspace": repo.workspace,
                    "repo_slug": repo.slug,
                }
            )
            yield self.item(entity_key=entity_key, generation_id=generation, **projected)
        yield self.complete(
            scope_parts=[self.name, repo.uuid],
            generation_id=generation,
            item_count=len(entity_keys),
            bucket_id=bucket_id,
            available=present,
            repository_uuid=repo.uuid,
            workspace_uuid=repo.workspace_uuid,
            workspace=repo.workspace,
            repo_slug=repo.slug,
        )

    def enabled(self, repo) -> bool:
        return True

    def include(self, record: Mapping[str, Any]) -> bool:
        return True

    def project(self, record: Mapping[str, Any]) -> Mapping[str, Any]:
        return {}

    def get_json_schema(self) -> Mapping[str, Any]:
        return schema(
            {
                "workspace": {"type": ["null", "string"]},
                "repo_slug": {"type": ["null", "string"]},
                "created_on": {"type": ["null", "string"]},
                "updated_on": {"type": ["null", "string"]},
            },
            additional=True,
        )


class EnvironmentsStream(RepositorySnapshotStream):
    name = "environments"
    resource = "environments"


class TagsStream(RepositorySnapshotStream):
    name = "tags"
    resource = "refs/tags"

    def include(self, record: Mapping[str, Any]) -> bool:
        # The tag's own date, not `target`'s: target is the commit it points at,
        # and a tag cut today can reference a years-old commit. A tag carrying
        # no date of its own is kept rather than judged by its commit.
        tagged_at = str(record.get("date") or "")
        if not self._start_date or not tagged_at:
            return True
        return tagged_at[:10] >= self._start_date

    def project(self, record: Mapping[str, Any]) -> Mapping[str, Any]:
        return {**record, "target_hash": (record.get("target") or {}).get("hash")}


class DeploymentsStream(RepositorySnapshotStream):
    name = "deployments"
    resource = "deployments"


def slim_pipeline(pipeline: Mapping[str, Any]) -> dict[str, Any]:
    """Only what the child streams read; the full object is never cached."""
    return {
        "uuid": pipeline.get("uuid"),
        "created_on": pipeline.get("created_on"),
        "completed_on": pipeline.get("completed_on"),
        "state": {"name": ((pipeline.get("state") or {}).get("name"))},
    }


class PipelineStateStream(BitbucketIncrementalStream):
    cursor_field = "created_on"

    # The stream emitting full pipeline records (pipelines) always fetches and
    # fills the slim cache; steps and test reports read it.
    reads_selection_cache = True

    def repository_records(self, repo, bucket_id: int):
        del bucket_id
        present, pipelines, new_state = self.pipeline_candidates(repo, self.repository_state(repo))
        if not present:
            return
        for pipeline in pipelines:
            yield from self.pipeline_records(repo, pipeline)
        self.commit_repository_state(repo, new_state)

    def pipeline_records(self, repo, pipeline: Mapping[str, Any]):
        raise NotImplementedError

    def pipeline_candidates(self, repo, prior: Mapping[str, Any]):
        cache_key = (repo_state_key(repo), str(prior.get("created_on") or ""))
        if self.reads_selection_cache:
            cached = self._catalog.pipeline_selections.get(cache_key)
            if cached is not None:
                present, slim, cached_state = cached
                return present, slim, dict(cached_state)
        present, pipelines, new_state = self._fetch_pipelines(repo, prior)
        self._catalog.pipeline_selections[cache_key] = (
            present,
            [slim_pipeline(p) for p in pipelines],
            dict(new_state),
        )
        return present, pipelines, new_state

    def _fetch_pipelines(self, repo, prior: Mapping[str, Any]):
        watermark = str(prior.get("created_on") or "")
        floor = None
        if watermark:
            floor = (datetime.fromisoformat(watermark.replace("Z", "+00:00")) - timedelta(minutes=5)).isoformat()
        elif self._start_date:
            floor = self._start_date
        params: dict[str, Any] = {"pagelen": "100", "sort": "created_on"}
        if floor:
            params["q"] = f'created_on>="{floor}"'
        pipelines: dict[str, Mapping[str, Any]] = {}
        present, records = self._client.paginate_optional(self._client.repo_path(repo, "pipelines"), params=params)
        if not present:
            return False, [], prior
        for pipeline in records:
            pipeline_uuid = str(pipeline.get("uuid") or "")
            if pipeline_uuid:
                pipelines[pipeline_uuid] = pipeline
        for pipeline_uuid in prior.get("open") or []:
            # 403 is tolerated here, not raised: Pipelines is a per-repository
            # feature, so a denial means "no pipelines visible", not "this
            # repository is unreadable". Letting it escape would mark the whole
            # repository inaccessible and suppress its commits and pull requests.
            response = self._client.request(
                "GET",
                self._client.repo_path(repo, f"pipelines/{pipeline_uuid}"),
                allow_statuses={403, 404},
            )
            if response is not None:
                pipeline = response.json()
                if isinstance(pipeline, Mapping):
                    pipelines[pipeline_uuid] = pipeline
        open_pipelines = [
            pipeline_uuid
            for pipeline_uuid, pipeline in pipelines.items()
            if ((pipeline.get("state") or {}).get("name") or "").upper() != "COMPLETED"
        ]
        new_state = {
            "created_on": max([watermark, *(str(pipeline.get("created_on") or "") for pipeline in pipelines.values())]),
            "open": sorted(open_pipelines),
        }
        return True, list(pipelines.values()), new_state


class PipelinesStream(PipelineStateStream):
    name = "pipelines"
    reads_selection_cache = False

    def pipeline_records(self, repo, pipeline: Mapping[str, Any]):
        pipeline_uuid = pipeline.get("uuid")
        entity_key = unique_key(self._tenant_id, self._source_id, *repo_scope(repo), pipeline_uuid)
        projected = dict(pipeline)
        projected.update(
            {
                "repository_uuid": repo.uuid,
                "workspace_uuid": repo.workspace_uuid,
                "workspace": repo.workspace,
                "repo_slug": repo.slug,
            }
        )
        yield self.item(entity_key=entity_key, **projected)

    def get_json_schema(self) -> Mapping[str, Any]:
        return schema(
            {
                "workspace": {"type": ["null", "string"]},
                "repo_slug": {"type": ["null", "string"]},
                "created_on": {"type": ["null", "string"]},
                "completed_on": {"type": ["null", "string"]},
            },
            additional=True,
        )


class PipelineStepsStream(PipelineStateStream):
    name = "pipeline_steps"

    def pipeline_records(self, repo, pipeline: Mapping[str, Any]):
        pipeline_uuid = pipeline.get("uuid")
        generation = self.generation(repo.uuid, pipeline_uuid, "steps")
        # Per-endpoint pagelen maxima are undocumented and inconsistent
        # (BCLOUD-13229); the steps endpoint is not verifiable against a public
        # repo. 50 is the lowest max observed across all probed endpoints, so it
        # is a safe floor here, and a pipeline has few steps regardless.
        present, steps = self._client.paginate_optional(
            self._client.repo_path(repo, f"pipelines/{pipeline_uuid}/steps"), params={"pagelen": "50"}
        )
        entity_keys: set[str] = set()
        for step in steps:
            step_uuid = step.get("uuid")
            entity_key = unique_key(self._tenant_id, self._source_id, *repo_scope(repo), pipeline_uuid, step_uuid)
            entity_keys.add(entity_key)
            projected = dict(step)
            projected.update(
                {
                    "repository_uuid": repo.uuid,
                    "workspace_uuid": repo.workspace_uuid,
                    "workspace": repo.workspace,
                    "repo_slug": repo.slug,
                    "pipeline_uuid": pipeline_uuid,
                    "pipeline_created_on": pipeline.get("created_on"),
                    "pipeline_completed_on": pipeline.get("completed_on"),
                }
            )
            yield self.item(entity_key=entity_key, generation_id=generation, **projected)
        yield self.complete(
            scope_parts=[repo.uuid, pipeline_uuid, "steps"],
            generation_id=generation,
            item_count=len(entity_keys),
            available=present,
            repository_uuid=repo.uuid,
            workspace_uuid=repo.workspace_uuid,
            workspace=repo.workspace,
            repo_slug=repo.slug,
            pipeline_uuid=pipeline_uuid,
            pipeline_created_on=pipeline.get("created_on"),
            pipeline_completed_on=pipeline.get("completed_on"),
        )

    def get_json_schema(self) -> Mapping[str, Any]:
        return PipelinesStream.get_json_schema(self)


class PipelineStepTestReportsStream(PipelineStateStream):
    name = "pipeline_step_test_reports"

    def pipeline_records(self, repo, pipeline: Mapping[str, Any]):
        pipeline_uuid = pipeline.get("uuid")
        # See PipelineStepsStream: steps pagelen is capped at the safe floor of
        # 50 because the endpoint's true maximum is unverifiable (BCLOUD-13229).
        _, steps = self._client.paginate_optional(
            self._client.repo_path(repo, f"pipelines/{pipeline_uuid}/steps"), params={"pagelen": "50"}
        )
        for step in steps:
            step_uuid = step.get("uuid")
            generation = self.generation(repo.uuid, pipeline_uuid, step_uuid, "test_reports")
            path = self._client.repo_path(repo, f"pipelines/{pipeline_uuid}/steps/{step_uuid}/test_reports")
            # Tolerates 403 for the same reason as the pipeline refetch above: a
            # feature-level denial must mark this snapshot unavailable, not the
            # repository. `available` below already records it.
            response = self._client.request("GET", path, allow_statuses={403, 404})
            count = 0
            if response is not None:
                payload = response.json()
                entity_key = unique_key(self._tenant_id, self._source_id, *repo_scope(repo), pipeline_uuid, step_uuid)
                count = 1
                yield self.item(
                    entity_key=entity_key,
                    generation_id=generation,
                    repository_uuid=repo.uuid,
                    workspace_uuid=repo.workspace_uuid,
                    workspace=repo.workspace,
                    repo_slug=repo.slug,
                    pipeline_uuid=pipeline_uuid,
                    step_uuid=step_uuid,
                    pipeline_created_on=pipeline.get("created_on"),
                    step_completed_on=step.get("completed_on"),
                    report=payload,
                )
            yield self.complete(
                scope_parts=[repo.uuid, pipeline_uuid, step_uuid, "test_reports"],
                generation_id=generation,
                item_count=count,
                available=response is not None,
                repository_uuid=repo.uuid,
                workspace_uuid=repo.workspace_uuid,
                workspace=repo.workspace,
                repo_slug=repo.slug,
                pipeline_uuid=pipeline_uuid,
                step_uuid=step_uuid,
                pipeline_created_on=pipeline.get("created_on"),
                step_completed_on=step.get("completed_on"),
            )

    def get_json_schema(self) -> Mapping[str, Any]:
        return schema(
            {
                "workspace": {"type": ["null", "string"]},
                "repo_slug": {"type": ["null", "string"]},
                "pipeline_uuid": {"type": ["null", "string"]},
                "step_uuid": {"type": ["null", "string"]},
                "pipeline_created_on": {"type": ["null", "string"]},
                "step_completed_on": {"type": ["null", "string"]},
                "report": {},
            },
            additional=True,
        )


def slim_issue(issue: Mapping[str, Any]) -> dict[str, Any]:
    return {"id": issue.get("id"), "updated_on": issue.get("updated_on")}


class IssueStateStream(BitbucketIncrementalStream):
    cursor_field = "updated_on"

    reads_selection_cache = True

    def repository_records(self, repo, bucket_id: int):
        del bucket_id
        if not repo.has_issues:
            self.commit_repository_state(repo, {})
            return
        present, issues, new_state = self.selected_issues(repo, self.repository_state(repo))
        if not present:
            return
        for issue in issues:
            yield from self.issue_records(repo, issue)
        self.commit_repository_state(repo, new_state)

    def issue_records(self, repo, issue: Mapping[str, Any]):
        raise NotImplementedError

    def selected_issues(self, repo, prior):
        cache_key = (repo_state_key(repo), str(prior.get("updated_on") or ""))
        if self.reads_selection_cache:
            cached = self._catalog.issue_selections.get(cache_key)
            if cached is not None:
                present, slim, cached_state = cached
                return present, slim, dict(cached_state)
        present, issues, new_state = self._fetch_issues(repo, prior)
        self._catalog.issue_selections[cache_key] = (
            present,
            [slim_issue(i) for i in issues],
            dict(new_state),
        )
        return present, issues, new_state

    def _fetch_issues(self, repo, prior):
        watermark = str(prior.get("updated_on") or "")
        floor = self._start_date
        if watermark:
            floor = (datetime.fromisoformat(watermark.replace("Z", "+00:00")) - timedelta(minutes=5)).isoformat()
        params = {"pagelen": "100", "sort": "updated_on"}
        if floor:
            params["q"] = f'updated_on>="{floor}"'
        present, records = self._client.paginate_optional(self._client.repo_path(repo, "issues"), params=params)
        if not present:
            return False, [], prior
        issues = list(records)
        return True, issues, {"updated_on": max([watermark, *(str(issue.get("updated_on") or "") for issue in issues)])}


class IssuesStream(IssueStateStream):
    name = "issues"
    reads_selection_cache = False

    def issue_records(self, repo, issue: Mapping[str, Any]):
        issue_id = issue.get("id")
        entity_key = unique_key(self._tenant_id, self._source_id, *repo_scope(repo), issue_id)
        projected = dict(issue)
        projected.update(
            {
                "repository_uuid": repo.uuid,
                "workspace_uuid": repo.workspace_uuid,
                "workspace": repo.workspace,
                "repo_slug": repo.slug,
            }
        )
        yield self.item(entity_key=entity_key, **projected)

    def get_json_schema(self) -> Mapping[str, Any]:
        return schema(
            {
                "workspace": {"type": ["null", "string"]},
                "repo_slug": {"type": ["null", "string"]},
                "created_on": {"type": ["null", "string"]},
                "updated_on": {"type": ["null", "string"]},
            },
            additional=True,
        )


class IssueChildStream(IssueStateStream):
    issue_resource = ""

    def issue_records(self, repo, issue: Mapping[str, Any]):
        issue_id = issue.get("id")
        generation = self.generation(repo.uuid, issue_id, self.issue_resource)
        present, records = self._client.paginate_optional(
            self._client.repo_path(repo, f"issues/{issue_id}/{self.issue_resource}"), params={"pagelen": "100"}
        )
        entity_keys: set[str] = set()
        for record in records:
            identity = record.get("id") or record.get("uuid")
            if identity is None:
                continue
            entity_key = unique_key(self._tenant_id, self._source_id, *repo_scope(repo), issue_id, identity)
            entity_keys.add(entity_key)
            projected = dict(record)
            projected.update(
                {
                    "repository_uuid": repo.uuid,
                    "workspace_uuid": repo.workspace_uuid,
                    "workspace": repo.workspace,
                    "repo_slug": repo.slug,
                    "issue_id": issue_id,
                    "issue_updated_on": issue.get("updated_on"),
                }
            )
            yield self.item(entity_key=entity_key, generation_id=generation, **projected)
        yield self.complete(
            scope_parts=[repo.uuid, issue_id, self.issue_resource],
            generation_id=generation,
            item_count=len(entity_keys),
            available=present,
            repository_uuid=repo.uuid,
            workspace_uuid=repo.workspace_uuid,
            workspace=repo.workspace,
            repo_slug=repo.slug,
            issue_id=issue_id,
            issue_updated_on=issue.get("updated_on"),
        )

    def get_json_schema(self) -> Mapping[str, Any]:
        return schema(
            {
                "workspace": {"type": ["null", "string"]},
                "repo_slug": {"type": ["null", "string"]},
                "issue_id": {"type": ["null", "integer"]},
                "issue_updated_on": {"type": ["null", "string"]},
            },
            additional=True,
        )


class IssueCommentsStream(IssueChildStream):
    name = "issue_comments"
    issue_resource = "comments"


class IssueChangesStream(IssueChildStream):
    name = "issue_changes"
    issue_resource = "changes"


class PRTasksStream(PullRequestStateStream):
    name = "pull_request_tasks"

    def pull_request_records(self, repo, pr: Mapping[str, Any]):
        pr_id = pr.get("id")
        revision = self.pull_request_revision(pr)
        generation = self.generation(repo.uuid, pr_id, "tasks")
        present, tasks = self._client.paginate_optional(
            self._client.repo_path(repo, f"pullrequests/{pr_id}/tasks"), params={"pagelen": "100"}
        )
        entity_keys: set[str] = set()
        for task in tasks:
            task_id = task.get("id")
            if task_id is None:
                continue
            entity_key = unique_key(self._tenant_id, self._source_id, *repo_scope(repo), pr_id, task_id)
            entity_keys.add(entity_key)
            projected = dict(task)
            projected.update(
                {
                    "repository_uuid": repo.uuid,
                    "workspace_uuid": repo.workspace_uuid,
                    "workspace": repo.workspace,
                    "repo_slug": repo.slug,
                    "pull_request_id": pr_id,
                    "pull_request_updated_on": pr.get("updated_on"),
                    **revision,
                }
            )
            yield self.item(entity_key=entity_key, generation_id=generation, **projected)
        yield self.complete(
            scope_parts=[repo.uuid, pr_id, "tasks"],
            generation_id=generation,
            item_count=len(entity_keys),
            available=present,
            repository_uuid=repo.uuid,
            workspace_uuid=repo.workspace_uuid,
            workspace=repo.workspace,
            repo_slug=repo.slug,
            pull_request_id=pr_id,
            pull_request_updated_on=pr.get("updated_on"),
            **revision,
        )

    def get_json_schema(self) -> Mapping[str, Any]:
        return schema(
            {
                "workspace": {"type": ["null", "string"]},
                "repo_slug": {"type": ["null", "string"]},
                "pull_request_id": {"type": ["null", "integer"]},
                "pull_request_updated_on": {"type": ["null", "string"]},
                "pull_request_source_commit_hash": {"type": ["null", "string"]},
                "pull_request_destination_commit_hash": {"type": ["null", "string"]},
            },
            additional=True,
        )
