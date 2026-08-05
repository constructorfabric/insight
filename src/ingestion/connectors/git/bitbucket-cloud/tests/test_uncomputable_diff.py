from __future__ import annotations

import json

import pytest

from source_bitbucket_cloud.client import UNCOMPUTABLE_DIFF, BitbucketApiError, BitbucketClient
from source_bitbucket_cloud.streams.base import BUCKET_COUNT
from source_bitbucket_cloud.streams.pr_diffstat import PRDiffstatStream
from tests.conftest import SHARED, FakeCatalog, repository


def read_all_buckets(stream):
    records = []
    for bucket in range(BUCKET_COUNT):
        records.extend(stream.read_records(None, stream_slice={"bucket_id": bucket}))
    return records


NO_COMMON_ANCESTOR = json.dumps({"type": "error", "error": {"message": "No common ancestor"}})
MALFORMED_REQUEST = json.dumps({"type": "error", "error": {"message": "Invalid pagelen"}})


class FakeResponse:
    def __init__(self, status_code: int, url: str, payload, text: str = ""):
        self.status_code = status_code
        self.url = url
        self.text = text
        self._payload = payload

    def json(self):
        return self._payload


class FakeSession:
    def __init__(self, routes):
        self.headers: dict[str, str] = {}
        self._routes = routes
        self.urls: list[str] = []

    def request(self, method, url, params=None, data=None, timeout=None):
        del method, params, data, timeout
        self.urls.append(url)
        for fragment, response in self._routes:
            if fragment in url:
                return FakeResponse(response[0], url, response[1], response[2])
        raise AssertionError(f"unrouted request: {url}")


def client_with(routes) -> BitbucketClient:
    client = BitbucketClient("token")
    client._session = FakeSession(routes)
    return client


def pull_request(pr_id: int = 42):
    return {
        "id": pr_id,
        "updated_on": "2026-06-30T00:00:00+00:00",
        "created_on": "2026-06-01T00:00:00+00:00",
        "state": "MERGED",
        "source": {"branch": {"name": "f"}, "commit": {"hash": "src"}},
        "destination": {"branch": {"name": "main"}, "commit": {"hash": "dst"}},
    }


class TestClientTolerance:
    def test_uncomputable_diff_reads_as_unavailable(self):
        client = client_with([("diffstat", (400, None, NO_COMMON_ANCESTOR))])

        present, entries = client.paginate_optional("x/diffstat", tolerate_messages=UNCOMPUTABLE_DIFF)

        assert present is False
        assert list(entries) == []

    def test_other_400_still_raises(self):
        client = client_with([("diffstat", (400, None, MALFORMED_REQUEST))])

        with pytest.raises(BitbucketApiError):
            client.paginate_optional("x/diffstat", tolerate_messages=UNCOMPUTABLE_DIFF)

    def test_tolerance_is_opt_in(self):
        client = client_with([("diffstat", (400, None, NO_COMMON_ANCESTOR))])

        with pytest.raises(BitbucketApiError):
            client.paginate_optional("x/diffstat")

    @pytest.mark.parametrize("body", ["", "<html>gateway</html>", json.dumps({"error": "flat"})])
    def test_unparseable_bodies_have_no_message(self, body):
        assert BitbucketApiError(400, "u", body).error_message == "", f"should not parse: {body!r}"


class TestDiffstatStreamTolerance:
    def build(self, diffstat_response):
        repo = repository()
        client = client_with(
            [
                ("diffstat", diffstat_response),
                ("pullrequests", (200, {"values": [pull_request()]}, "")),
            ]
        )
        stream = PRDiffstatStream(**{**SHARED, "client": client, "catalog": FakeCatalog([repo], client)})
        stream.state = {}
        return stream, repo

    def test_pull_request_without_common_ancestor_marks_the_snapshot_unavailable(self):
        stream, _ = self.build((400, None, NO_COMMON_ANCESTOR))

        records = read_all_buckets(stream)

        markers = [r for r in records if r["record_type"] == "snapshot_complete"]
        assert markers and markers[0]["snapshot_available"] is False, (
            "an undefined diff must read as 'could not look', not as an empty change set"
        )
        assert not [r for r in records if r["record_type"] == "item"]

    def test_other_400_still_fails_the_repository(self):
        stream, _ = self.build((400, None, MALFORMED_REQUEST))

        with pytest.raises(RuntimeError, match="repositories failed"):
            read_all_buckets(stream)

        assert stream._failed_repositories == ["ws/repo"], (
            "a 400 we do not recognise is a bug, not a permanent API answer"
        )
