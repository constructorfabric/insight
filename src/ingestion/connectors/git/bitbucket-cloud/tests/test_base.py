from __future__ import annotations

import pytest
from airbyte_cdk.models import SyncMode
from conftest import FakeCatalog, repository
from source_bitbucket_cloud.client import BitbucketApiError, BitbucketClient
from source_bitbucket_cloud.streams.base import (
    BUCKET_COUNT,
    normalize_start_date,
    now_iso,
    repository_bucket,
    truncate,
    unique_key,
)


def test_helpers():
    assert now_iso().endswith("Z")
    assert normalize_start_date("2026-06-30T12:34:56Z") == "2026-06-30"
    assert normalize_start_date(None) is None
    with pytest.raises(ValueError):
        normalize_start_date("invalid")
    assert truncate(None) is None
    assert len(truncate("x" * 20_000).encode()) <= 16_384
    assert unique_key("T", "S", "a:b") == "T:S:a%3Ab"
    assert 0 <= repository_bucket("{r-1}") < BUCKET_COUNT


def test_bucket_slices_and_repository_lookup(repositories_stream, repo):
    slices = list(repositories_stream.stream_slices(sync_mode=SyncMode.full_refresh))
    assert slices == [{"bucket_id": value} for value in range(BUCKET_COUNT)]
    selected = [item for slice_ in slices for item in repositories_stream.repositories_for_slice(slice_)]
    assert selected == [repo]


def test_incremental_state_is_versioned_and_pruned(commits_stream):
    current = repository(uuid="{current}")
    stale = repository(uuid="{stale}")
    while repository_bucket(stale.uuid) != repository_bucket(current.uuid):
        stale = repository(uuid=stale.uuid + "x")
    commits_stream._catalog = FakeCatalog([current])
    commits_stream.state = {"legacy": True}
    assert commits_stream.state == {"version": 2, "bucket_count": BUCKET_COUNT, "repositories": {}}
    commits_stream.commit_repository_state(current, {"head_shas": ["a"]})
    commits_stream.commit_repository_state(stale, {"head_shas": ["b"]})
    commits_stream.prune_bucket_state(repository_bucket(current.uuid), [current])
    assert commits_stream.state["repositories"] == {current.uuid: {"head_shas": ["a"]}}


def test_items_and_completion_have_stable_storage_keys(repositories_stream):
    item = repositories_stream.item(entity_key="T:S:e", generation_id="g", value=1)
    completion = repositories_stream.complete(scope_parts=["scope"], generation_id="g", item_count=1)
    assert item["unique_key"] == "T:S:e:g"
    assert item["record_type"] == "item"
    assert completion["record_type"] == "snapshot_complete"
    assert completion["snapshot_item_count"] == 1


class Response:
    def __init__(self, payload, status_code=200, url="https://api.bitbucket.org/2.0/x"):
        self.payload = payload
        self.status_code = status_code
        self.url = url
        self.text = str(payload)
        self.headers = {}

    def json(self):
        return self.payload


def test_client_retries_transient_status(monkeypatch):
    client = BitbucketClient("token")
    responses = [Response({}, 429), Response({"ok": True})]
    client._session.request = lambda *args, **kwargs: responses.pop(0)
    monkeypatch.setattr("source_bitbucket_cloud.client.time.sleep", lambda _: None)
    monkeypatch.setattr("source_bitbucket_cloud.client.random.random", lambda: 0)
    assert client.request("GET", "x").json() == {"ok": True}


def test_client_raises_typed_terminal_error():
    client = BitbucketClient("token")
    client._session.request = lambda *args, **kwargs: Response({"error": True}, 403)
    with pytest.raises(BitbucketApiError) as error:
        client.request("GET", "x")
    assert error.value.status_code == 403


def test_client_pagination_follows_next_and_detects_loops():
    client = BitbucketClient("token")
    pages = {
        "first": Response({"values": [{"id": 1}], "next": "second"}, url="first"),
        "second": Response({"values": [{"id": 2}]}, url="second"),
    }
    client.request = lambda method, path, **kwargs: pages[path]
    assert list(client.paginate("first")) == [{"id": 1}, {"id": 2}]
    pages["second"] = Response({"next": "first"}, url="second")
    with pytest.raises(RuntimeError, match="pagination loop"):
        list(client.paginate("first"))
