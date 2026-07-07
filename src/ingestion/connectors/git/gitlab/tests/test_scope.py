from __future__ import annotations

from source_gitlab.streams import concurrency
from source_gitlab.streams.concurrency import RequestGate
from source_gitlab.streams.issues import IssuesStream
from source_gitlab.streams.scope import (
    advance_cursor,
    compute_floor,
    scope_bases,
    scope_key,
    scope_params,
    scope_path,
)
from tests.conftest import BASE, FakeParent, fake_walk_window_yielding


class TestScopeBases:
    def test_instance_mode_when_unscoped(self):
        assert list(scope_bases((), (), parent=None)) == [{"mode": "instance"}]

    def test_project_mode_enumerates_unique_parent_projects(self):
        parent = FakeParent([
            ({"mode": "group", "group": "A"}, [{"id": 1}, {"id": 2}]),
            ({"mode": "group", "group": "B"}, [{"id": 2}, "junk", {"no_id": 1}]),
        ])
        bases = list(scope_bases(("A", "B"), (), parent=parent))
        assert bases == [
            {"mode": "project", "project": 1},
            {"mode": "project", "project": 2},
        ]


class TestScopeKeyAndPath:
    def test_key_instance(self):
        assert scope_key(None) == "instance"
        assert scope_key({"mode": "instance"}) == "instance"

    def test_key_project(self):
        assert scope_key({"mode": "project", "project": 7}) == "project:7"

    def test_path_instance(self):
        assert scope_path("issues", None) == "issues"

    def test_path_project_quoted(self):
        assert scope_path("issues", {"mode": "project", "project": "grp/app"}) == (
            "projects/grp%2Fapp/issues"
        )


class TestAdvanceCursor:
    def test_none_is_noop(self):
        state = {}
        advance_cursor(state, None)
        assert state == {}

    def test_sets_first_value(self):
        state = {}
        advance_cursor(state, "2026-06-01T00:00:00Z")
        assert state["updated_at"] == "2026-06-01T00:00:00Z"

    def test_keeps_max(self):
        state = {"updated_at": "2026-06-05T00:00:00Z"}
        advance_cursor(state, "2026-06-01T00:00:00Z")
        assert state["updated_at"] == "2026-06-05T00:00:00Z"
        advance_cursor(state, "2026-06-09T00:00:00Z")
        assert state["updated_at"] == "2026-06-09T00:00:00Z"


class TestComputeFloor:
    def test_no_watermark_uses_start(self):
        assert compute_floor("2026-06-01T00:00:00Z", None) == "2026-06-01T00:00:00Z"
        assert compute_floor(None, None) is None

    def test_watermark_overlapped_by_one_minute(self):
        assert compute_floor(None, "2026-06-01T10:00:00Z") == "2026-06-01T09:59:00Z"

    def test_start_wins_when_after_overlap(self):
        assert compute_floor(
            "2026-06-15T00:00:00Z", "2026-06-01T10:00:00Z",
        ) == "2026-06-15T00:00:00Z"


class TestScopeParams:
    def test_instance_adds_scope_all(self):
        params = scope_params(100, None)
        assert params["scope"] == "all"
        assert params["order_by"] == "updated_at"
        assert params["sort"] == "asc"
        assert params["per_page"] == 100

    def test_project_mode_no_scope(self):
        params = scope_params(50, {"mode": "project", "project": 1})
        assert "scope" not in params

    def test_window_bounds_passed_through(self):
        params = scope_params(50, {
            "mode": "instance",
            "updated_after": "2026-06-01T00:00:00Z",
            "updated_before": "2026-06-02T00:00:00Z",
        })
        assert params["updated_after"] == "2026-06-01T00:00:00Z"
        assert params["updated_before"] == "2026-06-02T00:00:00Z"


def _issues(start_date=None) -> IssuesStream:
    return IssuesStream(
        parent=FakeParent([]), gate=RequestGate(1), groups=(), projects=(),
        start_date=start_date, **BASE,
    )


class TestScopeUpdatedAtStream:
    def test_state_roundtrip(self):
        stream = _issues()
        assert stream.state == {}
        stream.state = {"scopes": {"instance": {"updated_at": "2026-06-01T00:00:00Z"}}}
        assert stream.state["scopes"]["instance"]["updated_at"] == "2026-06-01T00:00:00Z"
        stream.state = None
        assert stream.state == {}

    def test_stream_slices_carry_computed_floor(self):
        stream = _issues(start_date="2026-06-01T00:00:00Z")
        stream.state = {"scopes": {"instance": {"updated_at": "2026-06-10T00:00:00Z"}}}
        slices = list(stream.stream_slices())
        assert slices == [
            {"mode": "instance", "updated_after": "2026-06-09T23:59:00Z"},
        ]

    def test_stream_slices_without_state_fall_back_to_start(self):
        stream = _issues(start_date="2026-06-01T00:00:00Z")
        assert list(stream.stream_slices()) == [
            {"mode": "instance", "updated_after": "2026-06-01T00:00:00Z"},
        ]

    def test_read_records_envelopes_and_advances_cursor(self, monkeypatch):
        stream = _issues()
        raws = [
            {"project_id": 1, "iid": 5, "title": "t", "updated_at": "2026-06-02T00:00:00Z"},
            {"project_id": 1, "iid": 6, "title": "t2", "updated_at": "2026-06-03T00:00:00Z"},
        ]
        fake = fake_walk_window_yielding(raws)
        monkeypatch.setattr(concurrency, "walk_window", fake)
        records = list(stream.read_records(
            sync_mode=None, stream_slice={"mode": "instance", "updated_after": None},
        ))
        assert [r["iid"] for r in records] == [5, 6]
        assert records[0]["unique_key"] == "T:S:1:5"
        assert records[0]["data_source"] == "insight_gitlab"
        assert stream.state["scopes"]["instance"]["updated_at"] == "2026-06-03T00:00:00Z"
        # walk_window got the resource path + ordered params
        assert fake.calls[0]["path"] == "issues"
        assert fake.calls[0]["params"]["order_by"] == "updated_at"

    def test_read_records_project_scope_path(self, monkeypatch):
        stream = _issues()
        fake = fake_walk_window_yielding([])
        monkeypatch.setattr(concurrency, "walk_window", fake)
        list(stream.read_records(
            sync_mode=None, stream_slice={"mode": "project", "project": 9},
        ))
        assert fake.calls[0]["path"] == "projects/9/issues"
