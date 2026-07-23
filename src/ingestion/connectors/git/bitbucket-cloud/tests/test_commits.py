from airbyte_cdk.models import SyncMode
from conftest import branch
from source_bitbucket_cloud.streams.base import repository_bucket


def commit(sha="c1", date="2026-06-01T00:00:00+00:00", **extra):
    return {
        "hash": sha,
        "date": date,
        "message": "message",
        "author": {
            "raw": "Ann Author <ann@example.com>",
            "user": {"display_name": "Ann", "uuid": "{a}", "account_id": "aid"},
        },
        "committer": {"raw": "Build Bot"},
        "parents": [{"hash": "p1"}],
        **extra,
    }


def read(stream, repo):
    return list(stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": repository_bucket(repo.uuid)}))


def test_changed_heads_fetch_range_and_map_commit(commits_stream, client, repo):
    client.branch_values[repo.uuid] = [branch("main", "head"), branch("release", "head2")]
    client.commit_values = [commit()]
    records = read(commits_stream, repo)
    assert client.commit_calls == [(["head", "head2"], [])]
    assert len(records) == 1
    assert records[0]["hash"] == "c1"
    assert records[0]["author_name"] == "Ann Author"
    assert records[0]["author_email"] == "ann@example.com"
    assert records[0]["branch_name"] is None
    assert records[0]["parent_hashes"] == ["p1"]
    assert set(records[0]) <= set(commits_stream.get_json_schema()["properties"])
    assert commits_stream.state["repositories"][repo.uuid] == {"head_shas": ["head", "head2"]}


def test_unchanged_heads_make_no_commit_request(commits_stream, client, repo):
    client.branch_values[repo.uuid] = [branch("main", "head")]
    commits_stream.state = {"version": 2, "bucket_count": 8, "repositories": {repo.uuid: {"head_shas": ["head"]}}}
    assert read(commits_stream, repo) == []
    assert client.commit_calls == []


def test_changed_heads_exclude_previous_heads(commits_stream, client, repo):
    client.branch_values[repo.uuid] = [branch("main", "new")]
    commits_stream.state = {"version": 2, "bucket_count": 8, "repositories": {repo.uuid: {"head_shas": ["old"]}}}
    read(commits_stream, repo)
    assert client.commit_calls == [(["new"], ["old"])]


def test_start_date_filters_old_commits(commits_stream, client, repo):
    commits_stream._start_date = "2026-06-10"
    client.branch_values[repo.uuid] = [branch("main", "head")]
    client.commit_values = [commit(date="2026-06-01T00:00:00+00:00")]
    assert read(commits_stream, repo) == []


def test_message_truncation_and_raw_identity(commits_stream, repo):
    record = commits_stream._record(repo, commit(message="x" * 20_000, author={"raw": "buildbot", "user": None}))
    assert len(record["message"].encode()) <= 16_384
    assert record["author_name"] == "buildbot"
    assert record["author_email"] is None
