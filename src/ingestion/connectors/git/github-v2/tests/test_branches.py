from __future__ import annotations

import pytest
from source_github_v2.streams.branches import BranchesStream
from tests.conftest import SHARED, FakeRepoParent, FakeResponse

REPO_RECORD = {"owner": "acme", "name": "r1", "default_branch": "main", "pushed_at": "2026-01-01T00:00:00Z"}
SLICE = {"owner": "acme", "repo": "r1", "default_branch": "main", "pushed_at": "2026-01-01T00:00:00Z"}


def _stream(parent_records) -> BranchesStream:
    return BranchesStream(parent=FakeRepoParent(parent_records), **SHARED)


class TestSlicing:
    def test_slices_from_parent_child_records(self):
        stream = _stream([REPO_RECORD])
        assert list(stream.stream_slices()) == [SLICE]

    def test_incomplete_parent_records_skipped(self):
        stream = _stream([{"owner": "acme"}, {"name": "r1"}])
        assert list(stream.stream_slices()) == []

    def test_path_uses_owner_and_repo(self, branches_stream):
        assert branches_stream._path(stream_slice=SLICE) == "repos/acme/r1/branches"

    def test_path_without_owner_raises(self, branches_stream):
        with pytest.raises(ValueError, match="without owner/repo"):
            branches_stream._path(stream_slice={"repo": "r1"})


class TestParseResponse:
    def test_envelope_and_slice_context(self):
        stream = _stream([])
        payload = [{"name": "main", "commit": {"sha": "abc123"}}]
        records = list(stream.parse_response(FakeResponse(payload), stream_slice=SLICE))
        assert len(records) == 1
        rec = records[0]
        assert rec["unique_key"] == "T:S:acme:r1:main"
        assert rec["repo_owner"] == "acme"
        assert rec["repo_name"] == "r1"
        assert rec["default_branch_name"] == "main"
        assert rec["pushed_at"] == "2026-01-01T00:00:00Z"

    def test_single_object_payload_wrapped(self):
        stream = _stream([])
        records = list(stream.parse_response(FakeResponse({"name": "dev", "commit": {"sha": "d"}}), stream_slice=SLICE))
        assert len(records) == 1

    def test_error_status_yields_nothing(self):
        stream = _stream([])
        assert list(stream.parse_response(FakeResponse({}, status_code=409), stream_slice=SLICE)) == []

    def test_child_records_round_trip(self):
        stream = _stream([])
        payload = [
            {"name": "main", "commit": {"sha": "abc"}},
            {"name": "dev", "commit": None},  # missing HEAD tolerated
        ]
        list(stream.parse_response(FakeResponse(payload), stream_slice=SLICE))
        children = list(stream.get_child_records())
        assert children[0] == {
            "name": "main",
            "repo_owner": "acme",
            "repo_name": "r1",
            "default_branch": "main",
            "pushed_at": "2026-01-01T00:00:00Z",
            "commit": {"sha": "abc"},
        }
        assert children[1]["commit"] == {"sha": ""}

    def test_missing_file_yields_nothing(self, monkeypatch):
        stream = _stream([])
        stream._child_records_file.close()
        monkeypatch.setattr(stream, "_child_records_path", "/nonexistent/x.jsonl")
        assert list(stream.get_child_records()) == []


class TestSchema:
    def test_schema_has_slice_context_fields(self, branches_stream):
        props = branches_stream.get_json_schema()["properties"]
        for field in ("repo_owner", "repo_name", "default_branch_name", "pushed_at"):
            assert field in props
