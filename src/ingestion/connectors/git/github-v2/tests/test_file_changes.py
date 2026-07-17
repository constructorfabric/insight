from __future__ import annotations

import pytest
from airbyte_cdk.sources.streams.http import HttpStream
from source_github_v2.streams.base import GitHubAuthError
from source_github_v2.streams.file_changes import FileChangesStream
from tests.conftest import SHARED, FakeCommitsParent, FakeResponse

SLICE = {"owner": "acme", "repo": "r1", "sha": "c1", "committed_date": "2026-01-01T10:00:00Z"}


def _stream(tmp_path, rows) -> FileChangesStream:
    return FileChangesStream(parent=FakeCommitsParent(tmp_path, rows), **SHARED)


class TestSlicing:
    def test_one_slice_per_non_merge_commit(self, tmp_path):
        stream = _stream(
            tmp_path,
            [
                "c1\tacme\tr1\t2026-01-01T10:00:00Z\t1",
                "m1\tacme\tr1\t2026-01-02T10:00:00Z\t2",  # merge commit
                "c2\tacme\tr1\t2026-01-03T10:00:00Z\t0",  # root commit
            ],
        )
        slices = list(stream.stream_slices())
        assert [s["sha"] for s in slices] == ["c1", "c2"]
        assert slices[0] == SLICE

    def test_malformed_rows_ignored(self, tmp_path):
        stream = _stream(
            tmp_path,
            [
                "",  # blank line
                "c1\tacme\tr1",  # too few columns
                "c2\tacme\tr1\t2026-01-01T10:00:00Z\tnot-a-number",  # parent_count -> 0
            ],
        )
        slices = list(stream.stream_slices())
        assert [s["sha"] for s in slices] == ["c2"]

    def test_path_from_slice(self, tmp_path):
        stream = _stream(tmp_path, [])
        assert stream._path(stream_slice=SLICE) == "repos/acme/r1/commits/c1"

    def test_params_dropped_when_following_link_header(self, tmp_path):
        stream = _stream(tmp_path, [])
        assert stream.request_params() == {"per_page": "100"}
        assert stream.request_params(next_page_token={"next_url": "x"}) == {}


class TestParseResponse:
    def test_files_mapped_to_records(self, tmp_path):
        stream = _stream(tmp_path, [])
        payload = {
            "sha": "c1",
            "files": [
                {"filename": "a.py", "status": "modified", "additions": 1, "deletions": 2, "changes": 3, "patch": "@@"},
                {"filename": "b.py", "status": "renamed", "previous_filename": "old.py"},
            ],
        }
        records = list(stream.parse_response(FakeResponse(payload), stream_slice=SLICE))
        assert len(records) == 2
        assert records[0]["unique_key"] == "T:S:acme:r1:c1:a.py"
        assert records[0]["source_type"] == "commit"
        assert records[0]["committed_date"] == "2026-01-01T10:00:00Z"
        assert records[1]["previous_filename"] == "old.py"

    def test_no_files_key_yields_nothing(self, tmp_path):
        stream = _stream(tmp_path, [])
        assert list(stream.parse_response(FakeResponse({"sha": "c1"}), stream_slice=SLICE)) == []

    def test_error_status_yields_nothing(self, tmp_path):
        stream = _stream(tmp_path, [])
        assert list(stream.parse_response(FakeResponse({}, status_code=404), stream_slice=SLICE)) == []

    def test_401_raises_auth_error(self, tmp_path):
        stream = _stream(tmp_path, [])
        with pytest.raises(GitHubAuthError):
            list(stream.parse_response(FakeResponse({}, status_code=401, text="bad"), stream_slice=SLICE))


class TestReadRecords:
    def test_incomplete_slice_short_circuits(self, tmp_path):
        stream = _stream(tmp_path, [])
        assert list(stream.read_records(stream_slice={"owner": "acme"})) == []

    def test_delegates_to_cdk_for_complete_slice(self, tmp_path, monkeypatch):
        stream = _stream(tmp_path, [])

        def fake_read(self, sync_mode=None, stream_slice=None, stream_state=None, **kw):
            yield {"filename": "a.py"}

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        assert list(stream.read_records(stream_slice=SLICE)) == [{"filename": "a.py"}]

    def test_auth_error_propagates(self, tmp_path, monkeypatch):
        stream = _stream(tmp_path, [])

        def fake_read(self, **kw):
            raise GitHubAuthError("401")
            yield  # pragma: no cover

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        with pytest.raises(GitHubAuthError):
            list(stream.read_records(stream_slice=SLICE))

    def test_other_errors_logged_and_reraised(self, tmp_path, monkeypatch, caplog):
        stream = _stream(tmp_path, [])

        def fake_read(self, **kw):
            raise ValueError("boom")
            yield  # pragma: no cover

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        with caplog.at_level("ERROR", logger="airbyte"), pytest.raises(ValueError):
            list(stream.read_records(stream_slice=SLICE))
        assert "Failed file_changes for acme/r1/c1" in caplog.text


class TestSchema:
    def test_schema_has_file_fields(self, tmp_path):
        props = _stream(tmp_path, []).get_json_schema()["properties"]
        for field in ("filename", "status", "additions", "patch", "previous_filename"):
            assert field in props
