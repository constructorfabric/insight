from airbyte_cdk.models import SyncMode
from conftest import branch
from source_bitbucket_cloud.streams.base import repository_bucket


def read(stream, repo):
    return list(stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": repository_bucket(repo.uuid)}))


def test_file_changes_independently_walk_commits(file_changes_stream, client, repo):
    client.branch_values[repo.uuid] = [branch("main", "head")]
    client.commit_values = [{"hash": "c1", "date": "2026-06-01"}]
    path = client.repo_path(repo, "diffstat/c1")
    client.page_values[path] = [
        {
            "status": "renamed",
            "old": {"path": "old.py"},
            "new": {"path": "new.py"},
            "lines_added": 4,
            "lines_removed": 2,
        },
        {"status": "removed", "old": {"path": "gone.py"}, "lines_added": 0, "lines_removed": 8},
    ]
    records = read(file_changes_stream, repo)
    items, complete = records[:-1], records[-1]
    assert client.commit_calls == [(["head"], [])]
    assert [item["filename"] for item in items] == ["new.py", "gone.py"]
    assert items[0]["previous_filename"] == "old.py"
    assert complete["snapshot_item_count"] == 2
    assert complete["marker_type"] == "commit_snapshot_complete"
    assert set(items[0]) <= set(file_changes_stream.get_json_schema()["properties"])


def test_file_change_snapshot_counts_distinct_paths(file_changes_stream, client, repo):
    client.branch_values[repo.uuid] = [branch("main", "head")]
    client.commit_values = [{"hash": "c1", "date": "2026-06-01"}]
    path = client.repo_path(repo, "diffstat/c1")
    entry = {"status": "modified", "new": {"path": "a.py"}}
    client.page_values[path] = [entry, entry]
    records = read(file_changes_stream, repo)
    assert records[-1]["snapshot_item_count"] == 1


def test_unchanged_heads_do_not_refetch_diffs(file_changes_stream, client, repo):
    client.branch_values[repo.uuid] = [branch("main", "head")]
    file_changes_stream.state = {"version": 2, "bucket_count": 8, "repositories": {repo.uuid: {"head_shas": ["head"]}}}
    assert read(file_changes_stream, repo) == []
    assert client.commit_calls == []


def test_empty_diffstat_still_emits_completion(file_changes_stream, client, repo):
    records = list(file_changes_stream._diffstat(repo, "c1", "2026-06-01"))
    assert len(records) == 1
    assert records[0]["snapshot_item_count"] == 0
    assert list(file_changes_stream._diffstat(repo, "", None)) == []
