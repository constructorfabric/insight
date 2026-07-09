from __future__ import annotations

import pytest

from source_bitbucket_cloud.streams.repositories import RepositoriesStream
from tests.conftest import SHARED, FakeResponse


def _repo(slug="repo", updated_on="2026-06-01T00:00:00+00:00", **extra):
    base = {
        "slug": slug,
        "name": slug,
        "full_name": f"ws/{slug}",
        "uuid": "{r-1}",
        "is_private": True,
        "description": "d",
        "language": "python",
        "size": 10,
        "created_on": "2026-01-01T00:00:00+00:00",
        "updated_on": updated_on,
        "has_issues": False,
        "has_wiki": False,
        "mainbranch": {"name": "main"},
        "project": {"key": "PRJ", "name": "Project"},
    }
    base.update(extra)
    return base


class TestSlices:
    def test_one_slice_per_workspace_with_cursor(self):
        stream = RepositoriesStream(workspaces=["a", "b"], **SHARED)
        state = {"b": {"updated_on": "2026-05-01"}}
        slices = list(stream.stream_slices(stream_state=state))
        assert slices == [
            {"workspace": "a", "cursor_value": ""},
            {"workspace": "b", "cursor_value": "2026-05-01"},
        ]


class TestPathAndParams:
    def test_path(self, repositories_stream):
        assert repositories_stream._path({"workspace": "ws"}) == "repositories/ws"

    def test_path_requires_workspace(self, repositories_stream):
        with pytest.raises(ValueError):
            repositories_stream._path({})

    def test_params_first_run_no_filter(self, repositories_stream):
        params = repositories_stream.request_params(stream_slice={"cursor_value": ""})
        assert params["sort"] == "-updated_on"
        assert "q" not in params

    def test_params_cursor_becomes_q_filter(self, repositories_stream):
        params = repositories_stream.request_params(
            stream_slice={"cursor_value": "2026-05-01T00:00:00+00:00"},
        )
        assert params["q"] == 'updated_on>"2026-05-01T00:00:00+00:00"'

    def test_params_start_date_fallback(self):
        stream = RepositoriesStream(workspaces=["ws"], start_date="2026-04-01", **SHARED)
        params = stream.request_params(stream_slice={"cursor_value": ""})
        assert params["q"] == 'updated_on>"2026-04-01"'

    def test_params_cursor_wins_over_start_date(self):
        stream = RepositoriesStream(workspaces=["ws"], start_date="2026-04-01", **SHARED)
        params = stream.request_params(stream_slice={"cursor_value": "2026-05-01"})
        assert params["q"] == 'updated_on>"2026-05-01"'

    def test_params_empty_on_next_page(self, repositories_stream):
        assert repositories_stream.request_params(next_page_token={"next_url": "u"}) == {}


class TestParseResponse:
    def test_emits_enveloped_record(self, repositories_stream):
        payload = {"values": [_repo()]}
        slice_ = {"workspace": "ws", "cursor_value": ""}
        records = list(repositories_stream.parse_response(FakeResponse(payload), stream_slice=slice_))
        assert len(records) == 1
        rec = records[0]
        assert rec["unique_key"] == "T:S:ws:repo"
        assert rec["workspace"] == "ws"
        assert rec["mainbranch_name"] == "main"
        assert rec["project_key"] == "PRJ"
        assert rec["tenant_id"] == "T"
        assert rec["data_source"] == "insight_bitbucket_cloud"

    def test_skips_forks_when_enabled(self, repositories_stream):
        payload = {"values": [_repo(slug="fork", parent={"full_name": "up/stream"}), _repo()]}
        slice_ = {"workspace": "ws", "cursor_value": ""}
        records = list(repositories_stream.parse_response(FakeResponse(payload), stream_slice=slice_))
        assert [r["slug"] for r in records] == ["repo"]

    def test_keeps_forks_when_disabled(self):
        stream = RepositoriesStream(workspaces=["ws"], skip_forks=False, **SHARED)
        payload = {"values": [_repo(slug="fork", parent={"full_name": "up/stream"})]}
        records = list(stream.parse_response(FakeResponse(payload), stream_slice={"workspace": "ws"}))
        assert [r["slug"] for r in records] == ["fork"]

    def test_cursor_early_exit_stops_pagination(self, repositories_stream):
        payload = {"values": [
            _repo(slug="new", updated_on="2026-06-02T00:00:00+00:00"),
            _repo(slug="old", updated_on="2026-05-01T00:00:00+00:00"),
            _repo(slug="unreachable", updated_on="2026-04-01T00:00:00+00:00"),
        ]}
        slice_ = {"workspace": "ws", "cursor_value": "2026-05-15T00:00:00+00:00"}
        records = list(repositories_stream.parse_response(FakeResponse(payload), stream_slice=slice_))
        assert [r["slug"] for r in records] == ["new"]
        assert repositories_stream.next_page_token(FakeResponse({"next": "u"})) is None
        # flag resets after consumption
        assert repositories_stream.next_page_token(FakeResponse({"next": "u"})) == {"next_url": "u"}


class TestState:
    def test_advances_on_newer_cursor(self, repositories_stream):
        state = repositories_stream.get_updated_state(
            {}, {"workspace": "ws", "updated_on": "2026-06-01"},
        )
        assert state == {"ws": {"updated_on": "2026-06-01"}}

    def test_keeps_max_cursor(self, repositories_stream):
        state = {"ws": {"updated_on": "2026-06-05"}}
        out = repositories_stream.get_updated_state(
            state, {"workspace": "ws", "updated_on": "2026-06-01"},
        )
        assert out["ws"]["updated_on"] == "2026-06-05"

    def test_ignores_records_without_workspace_or_cursor(self, repositories_stream):
        assert repositories_stream.get_updated_state({}, {"updated_on": "2026-06-01"}) == {}
        assert repositories_stream.get_updated_state({}, {"workspace": "ws"}) == {}


class TestSchema:
    def test_schema_covers_all_record_fields(self, repositories_stream):
        payload = {"values": [_repo()]}
        record = next(iter(
            repositories_stream.parse_response(FakeResponse(payload),
                                               stream_slice={"workspace": "ws", "cursor_value": ""})
        ))
        schema_props = set(repositories_stream.get_json_schema()["properties"])
        assert set(record) <= schema_props
