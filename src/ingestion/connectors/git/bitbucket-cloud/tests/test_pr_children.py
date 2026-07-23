from source_bitbucket_cloud.streams.metric_events import IssuesStream, PipelinesStream
from source_bitbucket_cloud.streams.pr_activity import PRActivityStream
from source_bitbucket_cloud.streams.pr_diffstat import PRDiffstatStream


def pr():
    return {
        "id": 42,
        "updated_on": "2026-06-30T00:00:00+00:00",
        "source": {"commit": {"hash": "source"}},
        "destination": {"commit": {"hash": "dest"}},
    }


def test_comments_snapshot_uses_revision_and_distinct_count(pr_comments_stream, client, repo):
    path = client.repo_path(repo, "pullrequests/42/comments")
    value = {
        "id": 7,
        "user": {"uuid": "{u}", "account_id": "aid"},
        "content": {"raw": "looks good"},
        "inline": {"path": "a.py", "from": 1, "to": 2},
    }
    client.optional_values[path] = (True, [value, value])
    records = list(pr_comments_stream.pull_request_records(repo, pr()))
    assert records[0]["inline_path"] == "a.py"
    assert records[0]["pull_request_source_commit_hash"] == "source"
    assert records[-1]["pull_request_destination_commit_hash"] == "dest"
    assert records[-1]["snapshot_item_count"] == 1
    assert set(records[0]) <= set(pr_comments_stream.get_json_schema()["properties"])


def test_comments_unavailable_snapshot_is_marked(pr_comments_stream, client, repo):
    path = client.repo_path(repo, "pullrequests/42/comments")
    client.optional_values[path] = (False, [])
    record = list(pr_comments_stream.pull_request_records(repo, pr()))[-1]
    assert record["snapshot_available"] is False
    assert record["snapshot_item_count"] == 0


def test_pr_commits_snapshot_has_order_revision_and_distinct_count(pr_commits_stream, client, repo):
    path = client.repo_path(repo, "pullrequests/42/commits")
    value = {"hash": "c1", "author": {"user": {"uuid": "{a}"}}}
    client.optional_values[path] = (True, [value, value, {"author": {}}])
    records = list(pr_commits_stream.pull_request_records(repo, pr()))
    assert [record["commit_order"] for record in records[:-1]] == [0, 1]
    assert records[0]["pull_request_source_commit_hash"] == "source"
    assert records[-1]["snapshot_item_count"] == 1
    assert set(records[0]) <= set(pr_commits_stream.get_json_schema()["properties"])


def test_diffstat_snapshot_uses_final_revision_and_distinct_count(stream_args, client, repo):
    stream = PRDiffstatStream(**stream_args)
    path = client.repo_path(repo, "pullrequests/42/diffstat")
    entry = {
        "status": "modified",
        "old": {"path": "a.py"},
        "new": {"path": "a.py"},
        "lines_added": 5,
        "lines_removed": 2,
    }
    client.optional_values[path] = (True, [entry, entry])
    records = list(stream.pull_request_records(repo, pr()))
    assert records[0]["lines_added"] == 5
    assert records[0]["pull_request_destination_commit_hash"] == "dest"
    assert records[-1]["snapshot_item_count"] == 1


def test_activity_snapshot_uses_provider_event_timestamp(stream_args, client, repo):
    stream = PRActivityStream(**stream_args)
    path = client.repo_path(repo, "pullrequests/42/activity")
    client.optional_values[path] = (
        True,
        [{"update": {"state": "MERGED", "date": "2026-06-30T02:00:00+00:00", "author": {"account_id": "actor"}}}],
    )
    records = list(stream.pull_request_records(repo, pr()))
    assert records[0]["event_type"] == "update"
    assert records[0]["activity_date"] == "2026-06-30T02:00:00+00:00"
    assert records[0]["actor_account_id"] == "actor"
    assert records[-1]["snapshot_item_count"] == 1


def test_activity_classifies_every_event_type(stream_args, client, repo):
    stream = PRActivityStream(**stream_args)
    path = client.repo_path(repo, "pullrequests/42/activity")
    client.optional_values[path] = (
        True,
        [
            {"approval": {"date": "2026-06-30T03:00:00+00:00", "user": {"uuid": "u1"}}},
            {"comment": {"created_on": "2026-06-30T04:00:00+00:00", "user": {"uuid": "u2"}}},
            {"user": {"uuid": "u3"}},
        ],
    )
    records = list(stream.pull_request_records(repo, pr()))
    events = {record["event_type"]: record for record in records if record["record_type"] == "item"}
    assert set(events) == {"approval", "comment", "unknown"}
    assert events["approval"]["activity_date"] == "2026-06-30T03:00:00+00:00"
    assert events["comment"]["activity_date"] == "2026-06-30T04:00:00+00:00"
    assert records[-1]["snapshot_item_count"] == 3


def test_pipeline_cursor_uses_observed_provider_time(stream_args, client, repo):
    stream = PipelinesStream(**stream_args)
    path = client.repo_path(repo, "pipelines")
    client.optional_values[path] = (
        True,
        [{"uuid": "{pipeline}", "created_on": "2026-06-30T03:00:00+00:00", "state": {"name": "COMPLETED"}}],
    )
    present, pipelines, state = stream.pipeline_candidates(repo, {})
    assert present is True
    assert len(pipelines) == 1
    assert state == {"created_on": "2026-06-30T03:00:00+00:00", "open": []}


def test_empty_pipeline_and_issue_results_keep_provider_watermark(stream_args, client, repo):
    pipelines = PipelinesStream(**stream_args)
    issues = IssuesStream(**stream_args)
    client.optional_values[client.repo_path(repo, "pipelines")] = (True, [])
    client.optional_values[client.repo_path(repo, "issues")] = (True, [])
    pipeline_state = {"created_on": "2026-06-01T00:00:00+00:00"}
    issue_state = {"updated_on": "2026-06-02T00:00:00+00:00"}
    assert pipelines.pipeline_candidates(repo, pipeline_state)[2]["created_on"] == pipeline_state["created_on"]
    assert issues.selected_issues(repo, issue_state)[2] == issue_state
