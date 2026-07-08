"""BranchesStream transport behaviour (projection covered in test_projections)."""

from __future__ import annotations

from source_gitlab.streams import concurrency
from source_gitlab.streams.branches import BranchesStream
from source_gitlab.streams.concurrency import RequestGate
from tests.conftest import (
    BASE,
    FakeParent,
    fake_imap_bounded,
    fake_paginate_yielding,
)

RAW = {"name": "main", "commit": {"id": "sha1"}, "default": True}


def _stream(parent=None) -> BranchesStream:
    return BranchesStream(parent=parent or FakeParent([]), gate=RequestGate(1), **BASE)


class TestStreamSlices:
    def test_single_empty_slice(self):
        assert list(_stream().stream_slices()) == [{}]


class TestReadRecords:
    def test_explicit_parent_slice_paginates_directly(self, monkeypatch):
        stream = _stream()
        fake = fake_paginate_yielding([RAW])
        monkeypatch.setattr(concurrency, "paginate", fake)
        records = list(stream.read_records(
            sync_mode=None, stream_slice={"parent": {"id": 5}},
        ))
        assert records[0]["unique_key"] == "T:S:5:main"
        assert fake.calls[0]["path"] == "projects/5/repository/branches"
        assert fake.calls[0]["params"] == {"per_page": stream.page_size}
        assert fake.calls[0]["skippable"] == frozenset({404})

    def test_empty_slice_fans_out_over_unique_projects(self, monkeypatch):
        parent = FakeParent([
            ({"mode": "group", "group": "A"}, [{"id": 1}, {"id": 2}]),
            ({"mode": "group", "group": "B"}, [{"id": 2}]),  # dedup
        ])
        stream = _stream(parent=parent)
        fake = fake_paginate_yielding([RAW])
        monkeypatch.setattr(concurrency, "paginate", fake)
        monkeypatch.setattr(concurrency, "imap_bounded", fake_imap_bounded)
        records = list(stream.read_records(sync_mode=None, stream_slice={}))
        assert [r["unique_key"] for r in records] == ["T:S:1:main", "T:S:2:main"]
        assert [c["path"] for c in fake.calls] == [
            "projects/1/repository/branches",
            "projects/2/repository/branches",
        ]
