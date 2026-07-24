from airbyte_cdk.models import SyncMode
from conftest import FakeCatalog, repository
from source_bitbucket_cloud.streams.base import repository_bucket
from source_bitbucket_cloud.streams.repositories import RepositoriesStream


def test_repository_snapshot_maps_provider_fields(stream_args, repo):
    rich = repository(
        owner={"uuid": "{u}", "account_id": "aid", "display_name": "Ann"},
        project={"key": "PRJ", "name": "Project", "uuid": "{p}"},
        parent={"uuid": "{parent}", "full_name": "ws/upstream"},
        language="Python",
    )
    stream = RepositoriesStream(**{**stream_args, "catalog": FakeCatalog([rich])})
    records = list(stream.read_records(SyncMode.full_refresh, stream_slice={"bucket_id": repository_bucket(rich.uuid)}))
    item, complete = records
    assert item["repository_uuid"] == rich.uuid
    assert item["owner_account_id"] == "aid"
    assert item["project_key"] == "PRJ"
    assert item["parent_uuid"] == "{parent}"
    assert complete["snapshot_item_count"] == 1
    assert set(item) <= set(stream.get_json_schema()["properties"])


def test_snapshot_count_uses_distinct_entities(stream_args, repo):
    stream = RepositoriesStream(**{**stream_args, "catalog": FakeCatalog([repo, repo])})
    records = list(stream.read_records(SyncMode.full_refresh, stream_slice={"bucket_id": repository_bucket(repo.uuid)}))
    assert records[-1]["snapshot_item_count"] == 1
