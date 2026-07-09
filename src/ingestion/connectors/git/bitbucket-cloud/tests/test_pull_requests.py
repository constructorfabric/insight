from __future__ import annotations

import pytest

from source_bitbucket_cloud.streams.pull_requests import PullRequestsStream
from source_bitbucket_cloud.streams.repositories import RepositoriesStream
from tests.conftest import SHARED, FakeParent, FakeResponse


def _pr(pr_id=42, updated_on="2026-06-30T01:00:00+00:00", **extra):
    base = {
        "id": pr_id,
        "title": "Add feature",
        "description": "body",
        "state": "MERGED",
        "created_on": "2026-06-01T00:00:00+00:00",
        "updated_on": updated_on,
        "author": {"display_name": "Ann", "uuid": "{a-1}"},
        "source": {"branch": {"name": "feature"}},
        "destination": {"branch": {"name": "main"}},
        "merge_commit": {"hash": "abc123"},
        "comment_count": 3,
        "closed_by": {"display_name": "Mia", "uuid": "{u-1}"},
        "participants": [
            {
                "user": {"display_name": "Rev", "uuid": "{u-2}", "nickname": "rev"},
                "role": "REVIEWER",
                "approved": True,
                "state": "approved",
                "participated_on": "2026-06-30T00:55:00+00:00",
            },
        ],
    }
    base.update(extra)
    return base


SLICE = {"parent": {"workspace": "ws", "slug": "repo"}, "cursor_value": ""}


class TestPathAndParams:
    def test_path(self, pull_requests_stream):
        assert pull_requests_stream._path(SLICE) == "repositories/ws/repo/pullrequests"

    def test_path_requires_workspace_and_slug(self, pull_requests_stream):
        with pytest.raises(ValueError):
            pull_requests_stream._path({"parent": {"workspace": "ws"}})

    def test_params_request_participants_expansion(self, pull_requests_stream):
        params = pull_requests_stream.request_params()
        # Without this the list endpoint omits `participants` entirely.
        assert params["fields"] == "+values.participants"
        assert params["sort"] == "-updated_on"
        assert params["state"] == ["OPEN", "MERGED", "DECLINED", "SUPERSEDED"]

    def test_params_empty_on_next_page(self, pull_requests_stream):
        assert pull_requests_stream.request_params(next_page_token={"next_url": "u"}) == {}


class TestParseResponse:
    def test_terminal_actor_and_reviewer_timestamp_ingested(self, pull_requests_stream):
        records = list(pull_requests_stream.parse_response(
            FakeResponse({"values": [_pr()]}), stream_slice=SLICE,
        ))
        assert len(records) == 1
        rec = records[0]
        assert rec["unique_key"] == "T:S:ws:repo:42"
        assert rec["closed_by_display_name"] == "Mia"
        assert rec["closed_by_uuid"] == "{u-1}"
        participant = rec["participants"][0]
        assert participant["participated_on"] == "2026-06-30T00:55:00+00:00"
        assert participant["display_name"] == "Rev"
        assert participant["approved"] is True
        assert rec["merge_commit_hash"] == "abc123"
        assert rec["source_branch"] == "feature"
        assert rec["destination_branch"] == "main"

    def test_open_pr_without_closed_by_or_participants(self, pull_requests_stream):
        pr = _pr(state="OPEN", closed_by=None, participants=None, merge_commit=None)
        records = list(pull_requests_stream.parse_response(
            FakeResponse({"values": [pr]}), stream_slice=SLICE,
        ))
        rec = records[0]
        assert rec["closed_by_display_name"] is None
        assert rec["closed_by_uuid"] is None
        assert rec["participants"] == []
        assert rec["merge_commit_hash"] is None

    def test_participant_defaults_when_fields_missing(self, pull_requests_stream):
        pr = _pr(participants=[{"user": None}])
        records = list(pull_requests_stream.parse_response(
            FakeResponse({"values": [pr]}), stream_slice=SLICE,
        ))
        participant = records[0]["participants"][0]
        assert participant["approved"] is False
        assert participant["participated_on"] is None
        assert participant["display_name"] is None

    def test_description_truncated(self, pull_requests_stream):
        records = list(pull_requests_stream.parse_response(
            FakeResponse({"values": [_pr(description="x" * 5000)]}), stream_slice=SLICE,
        ))
        assert len(records[0]["description"].encode()) <= 1024

    def test_cursor_early_exit(self, pull_requests_stream):
        payload = {"values": [
            _pr(pr_id=2, updated_on="2026-06-30T00:00:00+00:00"),
            _pr(pr_id=1, updated_on="2026-06-01T00:00:00+00:00"),
        ]}
        slice_ = dict(SLICE, cursor_value="2026-06-15T00:00:00+00:00")
        records = list(pull_requests_stream.parse_response(FakeResponse(payload), stream_slice=slice_))
        assert [r["id"] for r in records] == [2]
        assert pull_requests_stream.next_page_token(FakeResponse({"next": "u"})) is None

    def test_start_date_cutoff(self):
        parent = RepositoriesStream(workspaces=["ws"], **SHARED)
        stream = PullRequestsStream(parent=parent, start_date="2026-06-15", **SHARED)
        payload = {"values": [_pr(pr_id=1, updated_on="2026-06-01T00:00:00+00:00")]}
        records = list(stream.parse_response(FakeResponse(payload), stream_slice=SLICE))
        assert records == []
        assert stream.next_page_token(FakeResponse({"next": "u"})) is None

    def test_schema_covers_all_record_fields(self, pull_requests_stream):
        record = next(iter(pull_requests_stream.parse_response(
            FakeResponse({"values": [_pr()]}), stream_slice=SLICE,
        )))
        schema_props = set(pull_requests_stream.get_json_schema()["properties"])
        assert set(record) <= schema_props


class TestSlices:
    def test_slices_carry_cursor_from_state(self, repositories_stream):
        stream = PullRequestsStream(parent=repositories_stream, **SHARED)
        stream.parent = FakeParent(records=[
            {"workspace": "ws", "slug": "repo"},
            {"workspace": "ws", "slug": "other"},
            {"workspace": "ws"},          # missing slug → skipped
            "not-a-mapping",              # non-mapping → skipped
        ])
        state = {"ws/repo": {"updated_on": "2026-06-01"}}
        slices = list(stream.stream_slices(sync_mode=None, stream_state=state))
        assert [s["partition_key"] for s in slices] == ["ws/repo", "ws/other"]
        assert slices[0]["cursor_value"] == "2026-06-01"
        assert slices[1]["cursor_value"] == ""


class TestState:
    def test_advances_per_repo(self, pull_requests_stream):
        record = {"workspace": "ws", "repo_slug": "repo", "updated_on": "2026-06-30"}
        state = pull_requests_stream.get_updated_state({}, record)
        assert state == {"ws/repo": {"updated_on": "2026-06-30"}}

    def test_keeps_max(self, pull_requests_stream):
        state = {"ws/repo": {"updated_on": "2026-07-01"}}
        out = pull_requests_stream.get_updated_state(
            state, {"workspace": "ws", "repo_slug": "repo", "updated_on": "2026-06-30"},
        )
        assert out["ws/repo"]["updated_on"] == "2026-07-01"

    def test_empty_cursor_ignored(self, pull_requests_stream):
        out = pull_requests_stream.get_updated_state(
            {}, {"workspace": "ws", "repo_slug": "repo", "updated_on": ""},
        )
        assert out == {}
