from __future__ import annotations

from source_bitbucket_cloud.streams.branches import BranchesStream
from tests.conftest import SHARED, FakeParent, FakeResponse


def _branch(name="feature", target_hash="aa11", target_date="2026-06-01T00:00:00+00:00"):
    return {"name": name, "target": {"hash": target_hash, "date": target_date}}


REPO = {
    "workspace": "ws",
    "slug": "repo",
    "mainbranch_name": "main",
    "updated_on": "2026-06-15T00:00:00+00:00",
}


def _slice(branch_heads=None, has_prior_state=None):
    heads = branch_heads or {}
    return {
        "parent": REPO,
        "branch_heads": heads,
        "has_prior_state": bool(heads) if has_prior_state is None else has_prior_state,
    }


class TestPathAndParams:
    def test_path(self, branches_stream):
        assert branches_stream._path(_slice()) == "repositories/ws/repo/refs/branches"

    def test_params_sort_desc_no_q_filter(self, branches_stream):
        params = branches_stream.request_params()
        assert params["sort"] == "-target.date"
        assert "q" not in params

    def test_params_empty_on_next_page(self, branches_stream):
        assert branches_stream.request_params(next_page_token={"next_url": "u"}) == {}


class TestSlices:
    def test_builds_branch_heads_from_state(self, branches_stream):
        branches_stream.parent = FakeParent(records=[REPO, {"workspace": "ws"}, 5])
        state = {
            "ws/repo/main": {"head_sha": "aa", "target_date": "2026-06-01"},
            "ws/repo/dev": {"head_sha": "bb"},
            "ws/other/main": {"head_sha": "cc"},
            "ws/repo/garbage": "not-a-dict",
        }
        slices = list(branches_stream.stream_slices(sync_mode=None, stream_state=state))
        assert len(slices) == 1
        assert slices[0]["branch_heads"] == {"main": "aa", "dev": "bb"}
        assert slices[0]["has_prior_state"] is True

    def test_no_state_no_heads(self, branches_stream):
        branches_stream.parent = FakeParent(records=[REPO])
        slices = list(branches_stream.stream_slices(sync_mode=None, stream_state={}))
        assert slices[0]["branch_heads"] == {}
        assert slices[0]["has_prior_state"] is False


class TestParseResponse:
    def test_emits_branch_with_default_flag(self, branches_stream):
        payload = {"values": [_branch(name="main"), _branch(name="feature")]}
        records = list(branches_stream.parse_response(FakeResponse(payload), stream_slice=_slice()))
        by_name = {r["name"]: r for r in records}
        assert by_name["main"]["is_default"] is True
        assert by_name["feature"]["is_default"] is False
        assert by_name["main"]["unique_key"] == "T:S:ws:repo:main"
        assert by_name["main"]["default_branch_name"] == "main"

    def test_head_unchanged_skip_does_not_stop_pagination(self, branches_stream):
        payload = {"values": [_branch(name="same", target_hash="aa11"), _branch(name="moved", target_hash="bb22")]}
        slice_ = _slice(branch_heads={"same": "aa11", "moved": "old"})
        records = list(branches_stream.parse_response(FakeResponse(payload), stream_slice=slice_))
        assert [r["name"] for r in records] == ["moved"]
        # pagination must continue: a force-pushed branch may be on a later page
        assert branches_stream.next_page_token(FakeResponse({"next": "u"})) == {"next_url": "u"}

    def test_start_date_cutoff_only_on_first_run(self):
        stream = BranchesStream(
            parent=FakeParent(records=[]), start_date="2026-06-10", **SHARED,
        )
        payload = {"values": [_branch(target_date="2026-06-01T00:00:00+00:00")]}
        # First run (no prior state) → cutoff fires, pagination stops.
        records = list(stream.parse_response(FakeResponse(payload), stream_slice=_slice()))
        assert records == []
        assert stream.next_page_token(FakeResponse({"next": "u"})) is None

    def test_no_cutoff_with_prior_state(self):
        stream = BranchesStream(
            parent=FakeParent(records=[]), start_date="2026-06-10", **SHARED,
        )
        payload = {"values": [_branch(target_date="2026-06-01T00:00:00+00:00")]}
        slice_ = _slice(branch_heads={"other": "zz"}, has_prior_state=True)
        records = list(stream.parse_response(FakeResponse(payload), stream_slice=slice_))
        assert len(records) == 1


class TestState:
    def test_stores_head_and_date_per_branch(self, branches_stream):
        record = {
            "workspace": "ws", "repo_slug": "repo", "name": "main",
            "target_hash": "aa11", "target_date": "2026-06-01",
        }
        state = branches_stream.get_updated_state({}, record)
        assert state == {"ws/repo/main": {"head_sha": "aa11", "target_date": "2026-06-01"}}

    def test_missing_identity_ignored(self, branches_stream):
        assert branches_stream.get_updated_state({}, {"name": "x"}) == {}

    def test_schema_covers_all_record_fields(self, branches_stream):
        payload = {"values": [_branch()]}
        record = next(iter(branches_stream.parse_response(FakeResponse(payload), stream_slice=_slice())))
        schema_props = set(branches_stream.get_json_schema()["properties"])
        assert set(record) <= schema_props
