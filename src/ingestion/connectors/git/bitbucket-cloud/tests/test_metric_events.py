from __future__ import annotations

from dataclasses import replace

from airbyte_cdk.models import SyncMode

from source_bitbucket_cloud.streams.base import repository_bucket
from source_bitbucket_cloud.streams.metric_events import (
    DeploymentsStream,
    EnvironmentsStream,
    IssueChangesStream,
    IssueCommentsStream,
    IssuesStream,
    PipelinesStream,
    PipelineStepsStream,
    PipelineStepTestReportsStream,
    PRTasksStream,
    TagsStream,
)
from tests.conftest import SHARED, FakeCatalog, FakeClient, repository

PIPELINE_PATH = "repositories/ws/repo/pipelines"
ISSUES_PATH = "repositories/ws/repo/issues"


def args(repo, client):
    return {**SHARED, "client": client, "catalog": FakeCatalog([repo], client)}


def read(stream, repo):
    return list(
        stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": repository_bucket(repo.uuid)})
    )


def items(records):
    return [record for record in records if record["record_type"] == "item"]


def marker(records):
    return next(record for record in records if record["record_type"] == "snapshot_complete")


class TestRepositorySnapshotStreams:
    def test_environments_emits_items_and_marker(self, repo):
        client = FakeClient()
        client.optional_values["repositories/ws/repo/environments"] = (True, [{"uuid": "e1", "name": "prod"}])
        records = read(EnvironmentsStream(**args(repo, client)), repo)
        assert items(records)[0]["entity_key"].endswith("e1")
        assert marker(records)["snapshot_item_count"] == 1
        assert marker(records)["snapshot_available"] is True

    def test_tags_project_adds_target_hash(self, repo):
        client = FakeClient()
        client.optional_values["repositories/ws/repo/refs/tags"] = (True, [{"name": "v1", "target": {"hash": "abc"}}])
        records = read(TagsStream(**args(repo, client)), repo)
        assert items(records)[0]["target_hash"] == "abc"

    def test_unavailable_resource_marks_available_false(self, repo):
        client = FakeClient()
        client.optional_values["repositories/ws/repo/deployments"] = (False, [])
        records = read(DeploymentsStream(**args(repo, client)), repo)
        assert items(records) == []
        assert marker(records)["snapshot_available"] is False
        assert marker(records)["snapshot_item_count"] == 0

    def test_record_without_identity_skipped(self, repo):
        client = FakeClient()
        client.optional_values["repositories/ws/repo/environments"] = (True, [{"description": "no id"}])
        records = read(EnvironmentsStream(**args(repo, client)), repo)
        assert items(records) == []
        assert marker(records)["snapshot_item_count"] == 0


class TestPipelines:
    def test_emits_items_and_advances_watermark(self, repo):
        client = FakeClient()
        client.optional_values[PIPELINE_PATH] = (
            True,
            [{"uuid": "p1", "created_on": "2026-06-02T00:00:00+00:00", "state": {"name": "COMPLETED"}}],
        )
        stream = PipelinesStream(**args(repo, client))
        stream.state = {}
        records = read(stream, repo)
        assert items(records)[0]["uuid"] == "p1"
        stored = stream.state["repositories"][repo.uuid]
        assert stored["created_on"] == "2026-06-02T00:00:00+00:00"
        assert stored["open"] == []

    def test_absent_pipelines_leaves_state_untouched(self, repo):
        client = FakeClient()
        client.optional_values[PIPELINE_PATH] = (False, [])
        stream = PipelinesStream(**args(repo, client))
        stream.state = {}
        assert read(stream, repo) == []
        assert stream.state["repositories"] == {}

    def test_open_pipeline_refetched_and_kept_open(self, repo):
        client = FakeClient()
        client.optional_values[PIPELINE_PATH] = (True, [])
        client.request_values["repositories/ws/repo/pipelines/open1"] = {
            "uuid": "open1",
            "created_on": "2026-06-01T00:00:00+00:00",
            "state": {"name": "IN_PROGRESS"},
        }
        stream = PipelinesStream(**args(repo, client))
        stream.state = {
            "version": 2,
            "bucket_count": 8,
            "repositories": {repo.uuid: {"created_on": "2026-06-01T00:00:00+00:00", "open": ["open1"]}},
        }
        records = read(stream, repo)
        assert items(records)[0]["uuid"] == "open1"
        assert stream.state["repositories"][repo.uuid]["open"] == ["open1"]


class TestPipelineChildren:
    def _pipeline(self, client):
        client.optional_values[PIPELINE_PATH] = (
            True,
            [{"uuid": "p1", "created_on": "2026-06-02T00:00:00+00:00", "state": {"name": "COMPLETED"}}],
        )

    def test_steps_snapshot(self, repo):
        client = FakeClient()
        self._pipeline(client)
        client.optional_values["repositories/ws/repo/pipelines/p1/steps"] = (True, [{"uuid": "s1"}])
        stream = PipelineStepsStream(**args(repo, client))
        stream.state = {}
        records = read(stream, repo)
        assert items(records)[0]["uuid"] == "s1"
        assert items(records)[0]["pipeline_uuid"] == "p1"
        assert marker(records)["snapshot_item_count"] == 1

    def test_test_reports_present(self, repo):
        client = FakeClient()
        self._pipeline(client)
        client.optional_values["repositories/ws/repo/pipelines/p1/steps"] = (True, [{"uuid": "s1"}])
        client.request_values["repositories/ws/repo/pipelines/p1/steps/s1/test_reports"] = {"count": 3}
        stream = PipelineStepTestReportsStream(**args(repo, client))
        stream.state = {}
        records = read(stream, repo)
        assert items(records)[0]["report"] == {"count": 3}
        assert marker(records)["snapshot_available"] is True

    def test_test_reports_absent(self, repo):
        client = FakeClient()
        self._pipeline(client)
        client.optional_values["repositories/ws/repo/pipelines/p1/steps"] = (True, [{"uuid": "s1"}])
        stream = PipelineStepTestReportsStream(**args(repo, client))
        stream.state = {}
        records = read(stream, repo)
        assert items(records) == []
        assert marker(records)["snapshot_available"] is False


class TestIssues:
    def test_disabled_tracker_commits_empty_state(self, repo):
        no_issues = replace(repo, has_issues=False)
        stream = IssuesStream(**args(no_issues, FakeClient()))
        stream.state = {}
        assert read(stream, no_issues) == []
        assert stream.state["repositories"][no_issues.uuid] == {}

    def test_emits_issues_and_watermark(self, repo):
        client = FakeClient()
        client.optional_values[ISSUES_PATH] = (True, [{"id": 7, "updated_on": "2026-06-05T00:00:00+00:00"}])
        stream = IssuesStream(**args(repo, client))
        stream.state = {}
        records = read(stream, repo)
        assert items(records)[0]["id"] == 7
        assert stream.state["repositories"][repo.uuid]["updated_on"] == "2026-06-05T00:00:00+00:00"

    def test_absent_issues_leaves_state_untouched(self, repo):
        client = FakeClient()
        client.optional_values[ISSUES_PATH] = (False, [])
        stream = IssuesStream(**args(repo, client))
        stream.state = {}
        read(stream, repo)
        assert stream.state["repositories"] == {}

    def test_issue_comments_snapshot(self, repo):
        client = FakeClient()
        client.optional_values[ISSUES_PATH] = (True, [{"id": 7, "updated_on": "2026-06-05T00:00:00+00:00"}])
        client.optional_values["repositories/ws/repo/issues/7/comments"] = (True, [{"id": 100}])
        records = read(IssueCommentsStream(**{**args(repo, client)}), repo)
        assert items(records)[0]["issue_id"] == 7
        assert items(records)[0]["id"] == 100
        assert marker(records)["snapshot_item_count"] == 1

    def test_issue_changes_snapshot(self, repo):
        client = FakeClient()
        client.optional_values[ISSUES_PATH] = (True, [{"id": 7, "updated_on": "2026-06-05T00:00:00+00:00"}])
        client.optional_values["repositories/ws/repo/issues/7/changes"] = (True, [{"id": 200}])
        records = read(IssueChangesStream(**args(repo, client)), repo)
        assert items(records)[0]["id"] == 200


class TestPRTasks:
    def test_tasks_snapshot(self, repo):
        client = FakeClient()
        client.pr_values = [{"id": 42, "updated_on": "2026-06-30T00:00:00+00:00", "state": "OPEN"}]
        client.optional_values["repositories/ws/repo/pullrequests/42/tasks"] = (True, [{"id": 5, "state": "UNRESOLVED"}])
        stream = PRTasksStream(**args(repo, client))
        stream.state = {}
        records = read(stream, repo)
        assert items(records)[0]["id"] == 5
        assert items(records)[0]["pull_request_id"] == 42
        assert marker(records)["snapshot_item_count"] == 1
