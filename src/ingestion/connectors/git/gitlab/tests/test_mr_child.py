"""MergeRequestChildStream machinery via the concrete notes stream."""

from __future__ import annotations

from source_gitlab.streams import concurrency
from source_gitlab.streams.concurrency import RequestGate
from source_gitlab.streams.merge_request_notes import MergeRequestNotesStream
from tests.conftest import (
    BASE,
    FakeParent,
    fake_imap_bounded,
    fake_paginate_yielding,
    fake_walk_window_yielding,
)


def _notes(start_date=None) -> MergeRequestNotesStream:
    return MergeRequestNotesStream(
        parent=FakeParent([]), gate=RequestGate(1), groups=(), projects=(),
        start_date=start_date, **BASE,
    )


MRS = [
    {"project_id": 1, "iid": 10, "updated_at": "2026-06-02T00:00:00Z"},
    {"project_id": 1, "iid": 11, "updated_at": "2026-06-03T00:00:00Z"},
    {"project_id": None, "iid": 12, "updated_at": "2026-06-04T00:00:00Z"},  # dropped
    {"project_id": 1, "iid": None, "updated_at": "2026-06-05T00:00:00Z"},   # dropped
]


class TestState:
    def test_roundtrip_and_scope_state(self):
        stream = _notes()
        assert stream.state == {}
        stream.state = None
        assert stream.state == {}
        entry = stream._scope_state("instance")
        entry["updated_at"] = "2026-06-01T00:00:00Z"
        assert stream.state["scopes"]["instance"]["updated_at"] == "2026-06-01T00:00:00Z"


class TestStreamSlices:
    def test_instance_mode(self):
        slices = list(_notes().stream_slices())
        assert slices == [{"scope_key": "instance", "base": {"mode": "instance"}}]


class TestMrTasks:
    def test_enumeration_filters_and_sequences(self, monkeypatch):
        stream = _notes(start_date="2026-06-01T00:00:00Z")
        fake = fake_walk_window_yielding(MRS)
        monkeypatch.setattr(concurrency, "walk_window", fake)
        tasks = list(stream._mr_tasks({"mode": "instance"}))
        assert tasks == [
            {"seq": 0, "project_id": 1, "mr_iid": 10,
             "mr_updated_at": "2026-06-02T00:00:00Z"},
            {"seq": 1, "project_id": 1, "mr_iid": 11,
             "mr_updated_at": "2026-06-03T00:00:00Z"},
        ]
        # enumeration hits the merge_requests listing with the start_date floor
        assert fake.calls[0]["path"] == "merge_requests"
        assert fake.calls[0]["base_slice"]["updated_after"] == "2026-06-01T00:00:00Z"

    def test_watermark_overlap_applied(self, monkeypatch):
        stream = _notes()
        stream.state = {"scopes": {"instance": {"updated_at": "2026-06-10T00:00:00Z"}}}
        fake = fake_walk_window_yielding([])
        monkeypatch.setattr(concurrency, "walk_window", fake)
        list(stream._mr_tasks({"mode": "instance"}))
        assert fake.calls[0]["base_slice"]["updated_after"] == "2026-06-09T23:59:00Z"


class TestFetchChild:
    def test_paginates_child_endpoint_and_envelopes(self, monkeypatch):
        stream = _notes()
        fake = fake_paginate_yielding([{"id": 100, "body": "hi"}])
        monkeypatch.setattr(concurrency, "paginate", fake)
        task = {"seq": 0, "project_id": 1, "mr_iid": 10,
                "mr_updated_at": "2026-06-02T00:00:00Z"}
        returned_task, records = stream._fetch_child(task)
        assert returned_task is task
        assert records[0]["unique_key"] == "T:S:1:100"
        assert records[0]["mr_iid"] == 10
        assert fake.calls[0]["path"] == "projects/1/merge_requests/10/notes"
        assert fake.calls[0]["params"] == {"per_page": stream.page_size}
        assert fake.calls[0]["skippable"] == frozenset({404})


class TestReadRecords:
    def test_empty_slice_yields_nothing(self):
        stream = _notes()
        assert list(stream.read_records(sync_mode=None, stream_slice={})) == []
        assert list(stream.read_records(sync_mode=None, stream_slice=None)) == []

    def test_end_to_end_advances_cursor_in_order(self, monkeypatch):
        stream = _notes()
        monkeypatch.setattr(concurrency, "walk_window", fake_walk_window_yielding(MRS))
        monkeypatch.setattr(
            concurrency, "paginate", fake_paginate_yielding([{"id": 100, "body": "x"}]),
        )
        monkeypatch.setattr(concurrency, "imap_bounded", fake_imap_bounded)
        records = list(stream.read_records(
            sync_mode=None,
            stream_slice={"scope_key": "instance", "base": {"mode": "instance"}},
        ))
        # one child record per surviving MR task
        assert [r["mr_iid"] for r in records] == [10, 11]
        # ordered-prefix watermark: both tasks completed → cursor at the max
        assert stream.state["scopes"]["instance"]["updated_at"] == "2026-06-03T00:00:00Z"


class TestOrderedPrefix:
    def test_out_of_order_completion_releases_prefix_only(self):
        prefix = concurrency.OrderedPrefix()
        assert list(prefix.complete(1, "b")) == []      # seq 0 still pending
        assert list(prefix.complete(0, "a")) == ["a", "b"]
        assert list(prefix.complete(2, "c")) == ["c"]
