from __future__ import annotations

import pytest
from source_github_v2.streams.repositories import RepositoriesStream
from tests.conftest import SHARED, FakeResponse


def _repo(name, owner="acme", archived=False, fork=False):
    return {
        "name": name,
        "owner": {"login": owner},
        "archived": archived,
        "fork": fork,
        "default_branch": "main",
        "pushed_at": "2026-01-01T00:00:00Z",
    }


class TestSlicing:
    def test_one_slice_per_org(self):
        stream = RepositoriesStream(organizations=["a", "b"], **SHARED)
        assert list(stream.stream_slices()) == [{"org": "a"}, {"org": "b"}]

    def test_path_uses_org(self, repositories_stream):
        assert repositories_stream._path(stream_slice={"org": "acme"}) == "orgs/acme/repos"

    def test_path_without_org_raises(self, repositories_stream):
        with pytest.raises(ValueError, match="without org"):
            repositories_stream._path(stream_slice={})

    def test_request_params_include_type_all(self, repositories_stream):
        assert repositories_stream.request_params() == {"per_page": "100", "type": "all"}


class TestParseResponse:
    def test_envelope_and_unique_key(self, repositories_stream):
        records = list(repositories_stream.parse_response(FakeResponse([_repo("r1")]), stream_slice={"org": "acme"}))
        assert len(records) == 1
        rec = records[0]
        assert rec["unique_key"] == "T:S:acme:r1"
        assert rec["repo_owner"] == "acme"
        assert rec["data_source"] == "insight_github"

    def test_archived_and_forks_skipped(self, repositories_stream):
        payload = [_repo("live"), _repo("old", archived=True), _repo("copy", fork=True)]
        records = list(repositories_stream.parse_response(FakeResponse(payload), stream_slice={"org": "acme"}))
        assert [r["name"] for r in records] == ["live"]

    def test_filters_can_be_disabled(self):
        stream = RepositoriesStream(organizations=["acme"], skip_archived=False, skip_forks=False, **SHARED)
        payload = [_repo("old", archived=True), _repo("copy", fork=True)]
        records = list(stream.parse_response(FakeResponse(payload), stream_slice={"org": "acme"}))
        assert len(records) == 2

    def test_single_object_payload_wrapped(self, repositories_stream):
        records = list(repositories_stream.parse_response(FakeResponse(_repo("solo")), stream_slice={"org": "acme"}))
        assert len(records) == 1

    def test_error_status_yields_nothing(self, repositories_stream):
        assert (
            list(repositories_stream.parse_response(FakeResponse({}, status_code=404), stream_slice={"org": "acme"}))
            == []
        )


class TestChildRecords:
    def test_round_trip_via_disk(self, repositories_stream):
        list(repositories_stream.parse_response(FakeResponse([_repo("r1"), _repo("r2")]), stream_slice={"org": "acme"}))
        children = list(repositories_stream.get_child_records())
        assert children == [
            {"owner": "acme", "name": "r1", "default_branch": "main", "pushed_at": "2026-01-01T00:00:00Z"},
            {"owner": "acme", "name": "r2", "default_branch": "main", "pushed_at": "2026-01-01T00:00:00Z"},
        ]

    def test_empty_when_nothing_parsed(self, repositories_stream):
        assert list(repositories_stream.get_child_records()) == []

    def test_missing_file_yields_nothing(self, repositories_stream, monkeypatch):
        repositories_stream._child_records_file.close()
        monkeypatch.setattr(repositories_stream, "_child_records_path", "/nonexistent/x.jsonl")
        assert list(repositories_stream.get_child_records()) == []


class TestSchema:
    def test_schema_has_envelope_fields(self, repositories_stream):
        props = repositories_stream.get_json_schema()["properties"]
        for field in ("tenant_id", "source_id", "unique_key", "repo_owner"):
            assert field in props
