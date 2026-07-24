from airbyte_cdk.models import SyncMode
from source_bitbucket_cloud.streams.base import repository_bucket


def pr(pr_id=42, updated_on="2026-06-30T01:00:00+00:00", **extra):
    return {
        "id": pr_id,
        "title": "Add feature",
        "description": "body",
        "state": "MERGED",
        "created_on": "2026-06-01T00:00:00+00:00",
        "updated_on": updated_on,
        "author": {"display_name": "Ann", "uuid": "{a}", "account_id": "aid"},
        "closed_by": {"display_name": "Mia", "uuid": "{m}"},
        "source": {"branch": {"name": "feature"}, "commit": {"hash": "source"}},
        "destination": {"branch": {"name": "main"}, "commit": {"hash": "dest"}},
        "merge_commit": {"hash": "merge"},
        "participants": [
            {
                "user": {"display_name": "Rev", "uuid": "{r}"},
                "role": "REVIEWER",
                "approved": True,
                "participated_on": "2026-06-30T00:00:00+00:00",
            }
        ],
        **extra,
    }


def read(stream, repo):
    return list(stream.read_records(SyncMode.incremental, stream_slice={"bucket_id": repository_bucket(repo.uuid)}))


def test_pull_request_maps_provider_identity_and_revisions(pull_requests_stream, client, repo):
    client.pr_values = [pr()]
    records = read(pull_requests_stream, repo)
    assert len(records) == 1
    record = records[0]
    assert record["source_commit_hash"] == "source"
    assert record["destination_commit_hash"] == "dest"
    assert record["closed_by_uuid"] == "{m}"
    assert record["participants"][0]["uuid"] == "{r}"
    assert set(record) <= set(pull_requests_stream.get_json_schema()["properties"])


def test_state_advances_only_to_observed_provider_timestamp(pull_requests_stream, client, repo):
    client.pr_values = [pr(updated_on="2026-06-30T01:00:00+00:00")]
    read(pull_requests_stream, repo)
    state = pull_requests_stream.state["repositories"][repo.uuid]
    assert state["updated_on"] == "2026-06-30T01:00:00+00:00"
    assert "2026-07" not in state["updated_on"]


def test_empty_provider_result_does_not_advance_clock(pull_requests_stream, client, repo):
    client.pr_values = []
    read(pull_requests_stream, repo)
    assert pull_requests_stream.state["repositories"][repo.uuid]["updated_on"] == ""


def test_existing_state_replays_open_and_terminal_prs(pull_requests_stream, client, repo):
    pull_requests_stream.state = {
        "version": 2,
        "bucket_count": 8,
        "repositories": {repo.uuid: {"updated_on": "2026-06-01T00:00:00+00:00", "reconcile_after_id": 0}},
    }
    client.pr_values = [pr(1, state="OPEN"), pr(2, state="MERGED")]
    records = read(pull_requests_stream, repo)
    assert {record["id"] for record in records} == {1, 2}


def test_description_is_bounded(pull_requests_stream, repo):
    record = pull_requests_stream._record(repo, pr(description="x" * 20_000))
    assert len(record["description"].encode()) <= 16_384
