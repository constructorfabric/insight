from __future__ import annotations

from source_gitlab.streams import concurrency
from source_gitlab.streams.concurrency import RequestGate
from source_gitlab.streams.file_changes import (
    CommitFileChangesStream,
    _DefaultHeadFrontier,
)
from tests.conftest import (
    BASE,
    FakeBranches,
    FakeParent,
    fake_imap_bounded,
    fake_paginate_yielding,
    fake_walk_window_yielding,
)


def _stream(parent=None, branches=None, start_date=None) -> CommitFileChangesStream:
    return CommitFileChangesStream(
        parent=parent or FakeParent([]),
        branches=branches or FakeBranches({}),
        gate=RequestGate(1),
        start_date=start_date,
        **BASE,
    )


class TestFrontier:
    def test_advances_only_after_enum_done_and_all_tasks_complete(self):
        stream = _stream()
        frontier = _DefaultHeadFrontier(stream)
        frontier.open(1, "H9")
        frontier.add_one(1)
        frontier.add_one(1)
        frontier.finish_enum(1)
        assert stream.state == {}          # 2 tasks still pending
        frontier.complete_one(1)
        assert stream.state == {}          # 1 task still pending
        frontier.complete_one(1)
        assert stream.state["projects"]["1"]["default_head"] == "H9"

    def test_zero_task_project_advances_at_enum_finish(self):
        stream = _stream()
        frontier = _DefaultHeadFrontier(stream)
        frontier.open(1, "H9")
        frontier.finish_enum(1)
        assert stream.state["projects"]["1"]["default_head"] == "H9"

    def test_complete_unknown_project_is_noop(self):
        frontier = _DefaultHeadFrontier(_stream())
        frontier.complete_one(42)          # never opened — must not raise

    def test_advance_happens_once(self):
        stream = _stream()
        frontier = _DefaultHeadFrontier(stream)
        frontier.open(1, "H9")
        frontier.finish_enum(1)
        stream.state["projects"]["1"]["default_head"] = "MUTATED"
        frontier.complete_one(1)           # advanced already → no overwrite
        assert stream.state["projects"]["1"]["default_head"] == "MUTATED"


class TestState:
    def test_roundtrip(self):
        stream = _stream()
        assert stream.state == {}
        stream.state = None
        assert stream.state == {}
        stream._project_state(1)["default_head"] = "H1"
        assert stream.state["projects"]["1"]["default_head"] == "H1"

    def test_stream_slices_single_empty(self):
        assert list(_stream().stream_slices()) == [{}]


class TestDiffTasks:
    def _project(self, pid=1):
        return {"id": pid, "default_branch": "main"}

    def test_fresh_project_uses_default_ref(self):
        parent = FakeParent([({"mode": "instance"}, [self._project()])])
        stream = _stream(
            parent=parent,
            branches=FakeBranches({1: [{"name": "main", "commit_sha": "H1"}]}),
        )
        seen_slices = []
        stream._iter_shas = lambda s: seen_slices.append(s) or iter(["sha1", "sha2"])
        frontier = _DefaultHeadFrontier(stream)
        tasks = list(stream._diff_tasks(frontier))
        assert tasks == [
            {"project_id": 1, "sha": "sha1"},
            {"project_id": 1, "sha": "sha2"},
        ]
        assert seen_slices == [{"project_id": 1, "ref": "main"}]
        # enum finished with 2 pending → head not yet advanced
        assert stream.state == {}

    def test_moved_head_uses_range_ref(self):
        parent = FakeParent([({"mode": "instance"}, [self._project()])])
        stream = _stream(
            parent=parent,
            branches=FakeBranches({1: [{"name": "main", "commit_sha": "H9"}]}),
        )
        stream.state = {"projects": {"1": {"default_head": "H1"}}}
        seen_slices = []
        stream._iter_shas = lambda s: seen_slices.append(s) or iter([])
        list(stream._diff_tasks(_DefaultHeadFrontier(stream)))
        assert seen_slices == [{"project_id": 1, "ref": "H1..H9"}]
        # zero diff tasks → head advanced at finish_enum
        assert stream.state["projects"]["1"]["default_head"] == "H9"

    def test_unchanged_head_skipped(self):
        parent = FakeParent([({"mode": "instance"}, [self._project()])])
        stream = _stream(
            parent=parent,
            branches=FakeBranches({1: [{"name": "main", "commit_sha": "H1"}]}),
        )
        stream.state = {"projects": {"1": {"default_head": "H1"}}}
        stream._iter_shas = lambda s: iter([])
        assert list(stream._diff_tasks(_DefaultHeadFrontier(stream))) == []

    def test_project_without_default_head_skipped(self):
        parent = FakeParent([({"mode": "instance"}, [
            self._project(), {"id": 2},  # no default_branch
        ])])
        stream = _stream(parent=parent, branches=FakeBranches({1: []}))
        stream._iter_shas = lambda s: iter([])
        assert list(stream._diff_tasks(_DefaultHeadFrontier(stream))) == []


class TestIterShas:
    def test_filters_merge_commits_and_missing_ids(self, monkeypatch):
        stream = _stream(start_date="2026-06-01T00:00:00Z")
        fake = fake_walk_window_yielding([
            {"id": "s1", "parent_ids": ["p1"]},
            {"id": "s2", "parent_ids": ["p1", "p2"]},   # merge → skipped
            {"id": None, "parent_ids": []},              # no id → skipped
            {"id": "s3", "parent_ids": []},              # root commit → kept
        ])
        monkeypatch.setattr(concurrency, "walk_window", fake)
        shas = list(stream._iter_shas({"project_id": 1, "ref": "main"}))
        assert shas == ["s1", "s3"]
        assert fake.calls[0]["base_slice"]["since"] == "2026-06-01T00:00:00Z"
        assert fake.calls[0]["path"] == "projects/1/repository/commits"

    def test_commit_params_with_until(self):
        stream = _stream()
        params = stream._commit_params({"ref": "main", "since": "s", "until": "u"})
        assert params == {
            "ref_name": "main", "per_page": stream.page_size, "since": "s", "until": "u",
        }

    def test_commit_min_projection(self):
        stream = _stream()
        assert stream._commit_min({"id": "x", "parent_ids": ["a"]}, {}) == {
            "id": "x", "parent_count": 1,
        }
        assert stream._commit_min({}, {}) == {"id": None, "parent_count": 0}


class TestFetchDiffAndProjection:
    TASK = {"project_id": 1, "sha": "sha1"}

    def test_fetch_diff_envelopes(self, monkeypatch):
        stream = _stream()
        fake = fake_paginate_yielding([{
            "old_path": "a.py", "new_path": "a.py", "new_file": False,
            "deleted_file": False, "renamed_file": False,
            "diff": "@@ -1,1 +1,2 @@\n+x\n-y\n",
        }])
        monkeypatch.setattr(concurrency, "paginate", fake)
        task, records = stream._fetch_diff(self.TASK)
        assert task is self.TASK
        rec = records[0]
        assert rec["unique_key"] == "T:S:1:sha1:a.py"
        assert rec["lines_added"] == 1
        assert rec["lines_removed"] == 1
        assert rec["diff_truncated"] is False
        assert fake.calls[0]["path"] == "projects/1/repository/commits/sha1/diff"

    def test_record_key_falls_back_to_old_path(self):
        stream = _stream()
        key = stream._record_key({"old_path": "gone.py"}, self.TASK)
        assert key == ["1", "sha1", "gone.py"]

    def test_projection_truncated_diff(self):
        stream = _stream()
        out = stream._project({"old_path": "a", "new_path": "a", "too_large": True}, self.TASK)
        assert out["lines_added"] is None
        assert out["diff_truncated"] is True


class TestReadRecordsEndToEnd:
    def test_streams_diffs_and_advances_head(self, monkeypatch):
        parent = FakeParent([({"mode": "instance"}, [{"id": 1, "default_branch": "main"}])])
        stream = _stream(
            parent=parent,
            branches=FakeBranches({1: [{"name": "main", "commit_sha": "H1"}]}),
        )
        stream._iter_shas = lambda s: iter(["sha1"])
        fake = fake_paginate_yielding([{
            "old_path": "a.py", "new_path": "a.py", "diff": "",
        }])
        monkeypatch.setattr(concurrency, "paginate", fake)
        monkeypatch.setattr(concurrency, "imap_bounded", fake_imap_bounded)
        records = list(stream.read_records(sync_mode=None))
        assert [r["unique_key"] for r in records] == ["T:S:1:sha1:a.py"]
        # the only diff task completed → default_head advanced
        assert stream.state["projects"]["1"]["default_head"] == "H1"
