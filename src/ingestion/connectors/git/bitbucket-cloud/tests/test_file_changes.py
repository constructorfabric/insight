from __future__ import annotations

import pytest

from source_bitbucket_cloud.streams.file_changes import FileChangesStream
from tests.conftest import SHARED, FakeResponse


class _FakeCommitsParent:
    """Commits-parent stub capturing the state file_changes translates in."""

    def __init__(self, slices=None, records=None):
        self._slices = slices or []
        self._records = records or []
        self.seen_state = None

    def stream_slices(self, sync_mode=None, cursor_field=None, stream_state=None):
        self.seen_state = stream_state
        yield from self._slices

    def read_records(self, **kwargs):
        yield from self._records


BRANCH = {
    "workspace": "ws", "repo_slug": "repo", "name": "main",
    "target_hash": "aa11",
}


def _slice(sha="c" * 40):
    return {
        "workspace": "ws", "slug": "repo", "branch": "main", "sha": sha,
        "committed_date": "2026-06-01T00:00:00+00:00",
        "partition_key": "ws/repo/main", "head_sha": "aa11",
    }


def _stream(parent=None) -> FileChangesStream:
    return FileChangesStream(parent=parent or _FakeCommitsParent(), **SHARED)


class TestPath:
    def test_path(self):
        assert _stream()._path(_slice()) == f"repositories/ws/repo/diffstat/{'c' * 40}"

    def test_path_requires_identity(self):
        with pytest.raises(ValueError):
            _stream()._path({"workspace": "ws", "slug": "repo"})


class TestTranslateState:
    def test_maps_committed_date_to_date(self):
        stream = _stream()
        state = {
            "ws/repo/main": {"committed_date": "2026-06-01", "head_sha": "aa"},
            "ws/repo/dev": {"committed_date": ""},
            "junk": "not-a-dict",
        }
        translated = stream._translate_state(state)
        assert translated == {
            "ws/repo/main": {"date": "2026-06-01", "head_sha": "aa"},
            "ws/repo/dev": {"date": "", "head_sha": ""},
        }

    def test_empty_state(self):
        assert _stream()._translate_state({}) == {}


class TestSlices:
    def test_translated_state_passed_to_parent_and_merges_skipped(self):
        parent = _FakeCommitsParent(
            slices=[{"parent": BRANCH}],
            records=[
                {"hash": "1" * 40, "date": "2026-06-01", "parent_hashes": ["p1"]},
                {"hash": "2" * 40, "date": "2026-06-02", "parent_hashes": ["p1", "p2"]},  # merge
                {"hash": "", "date": "2026-06-03", "parent_hashes": []},                  # no sha
            ],
        )
        stream = _stream(parent)
        state = {"ws/repo/main": {"committed_date": "2026-05-01", "head_sha": "old"}}
        slices = list(stream.stream_slices(sync_mode=None, stream_state=state))
        assert [s["sha"] for s in slices] == ["1" * 40]
        assert slices[0]["committed_date"] == "2026-06-01"
        assert slices[0]["head_sha"] == "aa11"
        assert slices[0]["partition_key"] == "ws/repo/main"
        # parent received commits-shaped state
        assert parent.seen_state == {"ws/repo/main": {"date": "2026-05-01", "head_sha": "old"}}


class TestParseResponse:
    def test_emits_diffstat_rows(self):
        stream = _stream()
        payload = {"values": [
            {"status": "modified", "new": {"path": "a.py"}, "old": {"path": "a.py"},
             "lines_added": 3, "lines_removed": 1},
        ]}
        records = list(stream.parse_response(FakeResponse(payload), stream_slice=_slice()))
        assert len(records) == 1
        rec = records[0]
        assert rec["unique_key"] == f"T:S:ws:repo:{'c' * 40}:a.py"
        assert rec["source_type"] == "commit"
        assert rec["additions"] == 3
        assert rec["deletions"] == 1
        assert rec["previous_filename"] is None

    def test_renamed_carries_previous_filename(self):
        stream = _stream()
        payload = {"values": [
            {"status": "renamed", "new": {"path": "new.py"}, "old": {"path": "old.py"},
             "lines_added": 0, "lines_removed": 0},
        ]}
        rec = next(iter(stream.parse_response(FakeResponse(payload), stream_slice=_slice())))
        assert rec["filename"] == "new.py"
        assert rec["previous_filename"] == "old.py"

    def test_removed_file_uses_old_path(self):
        stream = _stream()
        payload = {"values": [
            {"status": "removed", "new": None, "old": {"path": "gone.py"},
             "lines_added": 0, "lines_removed": 10},
        ]}
        rec = next(iter(stream.parse_response(FakeResponse(payload), stream_slice=_slice())))
        assert rec["filename"] == "gone.py"

    def test_entry_without_any_path_skipped(self):
        stream = _stream()
        payload = {"values": [{"status": "modified", "new": None, "old": None}]}
        assert list(stream.parse_response(FakeResponse(payload), stream_slice=_slice())) == []

    def test_incomplete_slice_emits_nothing(self):
        stream = _stream()
        payload = {"values": [{"new": {"path": "a.py"}}]}
        out = list(stream.parse_response(FakeResponse(payload), stream_slice={"workspace": "ws"}))
        assert out == []

    def test_schema_covers_all_record_fields(self):
        stream = _stream()
        payload = {"values": [
            {"status": "modified", "new": {"path": "a.py"}, "old": None,
             "lines_added": 1, "lines_removed": 0},
        ]}
        record = next(iter(stream.parse_response(FakeResponse(payload), stream_slice=_slice())))
        schema_props = set(stream.get_json_schema()["properties"])
        assert set(record) <= schema_props


class TestState:
    def test_state_keyed_by_current_partition(self):
        stream = _stream()
        # parse_response sets the current partition context
        list(stream.parse_response(FakeResponse({"values": []}), stream_slice=_slice()))
        record = {"committed_date": "2026-06-05T00:00:00+00:00"}
        state = stream.get_updated_state({}, record)
        assert state == {"ws/repo/main": {
            "committed_date": "2026-06-05T00:00:00+00:00", "head_sha": "aa11",
        }}

    def test_no_partition_context_is_noop(self):
        stream = _stream()
        assert stream.get_updated_state({}, {"committed_date": "2026-06-05"}) == {}

    def test_keeps_max_date(self):
        stream = _stream()
        list(stream.parse_response(FakeResponse({"values": []}), stream_slice=_slice()))
        state = {"ws/repo/main": {"committed_date": "2026-06-09"}}
        out = stream.get_updated_state(state, {"committed_date": "2026-06-05"})
        assert out["ws/repo/main"]["committed_date"] == "2026-06-09"
