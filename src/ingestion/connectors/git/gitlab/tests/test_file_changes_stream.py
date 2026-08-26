from __future__ import annotations

from source_gitlab.streams import concurrency
from source_gitlab.streams.concurrency import RequestGate
from source_gitlab.streams.file_changes import (
    CommitFileChangesStream,
    _HeadFrontier,
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
        frontier = _HeadFrontier(stream)
        frontier.open((1, None), "H9")
        frontier.add_one((1, None))
        frontier.add_one((1, None))
        frontier.finish_enum((1, None))
        assert stream.state == {}          # 2 tasks still pending
        frontier.complete_one((1, None))
        assert stream.state == {}          # 1 task still pending
        frontier.complete_one((1, None))
        assert stream.state["projects"]["1"]["default_head"] == "H9"

    def test_zero_task_project_advances_at_enum_finish(self):
        stream = _stream()
        frontier = _HeadFrontier(stream)
        frontier.open((1, None), "H9")
        frontier.finish_enum((1, None))
        assert stream.state["projects"]["1"]["default_head"] == "H9"

    def test_complete_unknown_project_is_noop(self):
        frontier = _HeadFrontier(_stream())
        frontier.complete_one((42, None))  # never opened — must not raise

    def test_advance_happens_once(self):
        stream = _stream()
        frontier = _HeadFrontier(stream)
        frontier.open((1, None), "H9")
        frontier.finish_enum((1, None))
        stream.state["projects"]["1"]["default_head"] = "MUTATED"
        frontier.complete_one((1, None))   # advanced already → no overwrite
        assert stream.state["projects"]["1"]["default_head"] == "MUTATED"

    def test_named_branch_advances_under_branches_not_default_head(self):
        stream = _stream()
        frontier = _HeadFrontier(stream)
        frontier.open((1, "feat"), "H2")
        frontier.finish_enum((1, "feat"))
        pstate = stream.state["projects"]["1"]
        assert pstate["branches"] == {"feat": "H2"}
        assert "default_head" not in pstate

    def test_one_branch_failing_leaves_the_others_advanced(self):
        stream = _stream()
        frontier = _HeadFrontier(stream)
        for name in ("good", "bad"):
            frontier.open((1, name), f"H-{name}")
            frontier.add_one((1, name))
            frontier.finish_enum((1, name))
        frontier.complete_one((1, "good"))
        # "bad" never completed its task, so its head must not move — the next
        # sync has to re-walk that range.
        assert stream.state["projects"]["1"]["branches"] == {"good": "H-good"}


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

    def _capture(self, stream, shas=()):
        """Record every (slice, skippable) _diff_tasks asks for."""
        seen = []

        def fake(enum_slice, *, skippable=frozenset()):
            seen.append((enum_slice, skippable))
            return iter(shas)

        stream._iter_shas = fake
        return seen

    def test_fresh_project_uses_default_ref(self):
        parent = FakeParent([({"mode": "instance"}, [self._project()])])
        stream = _stream(
            parent=parent,
            branches=FakeBranches({1: [{"name": "main", "commit_sha": "H1"}]}),
        )
        seen = self._capture(stream, ["sha1", "sha2"])
        frontier = _HeadFrontier(stream)
        tasks = list(stream._diff_tasks(frontier))
        assert tasks == [
            {"project_id": 1, "sha": "sha1", "ref_key": (1, None)},
            {"project_id": 1, "sha": "sha2", "ref_key": (1, None)},
        ]
        assert seen == [({"project_id": 1, "ref": "main"}, frozenset())]
        # enum finished with 2 pending → head not yet advanced
        assert stream.state == {}

    def test_moved_head_uses_range_ref(self):
        parent = FakeParent([({"mode": "instance"}, [self._project()])])
        stream = _stream(
            parent=parent,
            branches=FakeBranches({1: [{"name": "main", "commit_sha": "H9"}]}),
        )
        stream.state = {"projects": {"1": {"default_head": "H1"}}}
        seen = self._capture(stream)
        list(stream._diff_tasks(_HeadFrontier(stream)))
        assert seen == [({"project_id": 1, "ref": "H1..H9"}, frozenset())]
        # zero diff tasks → head advanced at finish_enum
        assert stream.state["projects"]["1"]["default_head"] == "H9"

    def test_unchanged_default_head_skips_only_the_default_walk(self):
        parent = FakeParent([({"mode": "instance"}, [self._project()])])
        stream = _stream(
            parent=parent,
            branches=FakeBranches({1: [
                {"name": "main", "commit_sha": "H1"},
                {"name": "feat", "commit_sha": "H2"},
            ]}),
        )
        stream.state = {"projects": {"1": {"default_head": "H1"}}}
        seen = self._capture(stream)
        list(stream._diff_tasks(_HeadFrontier(stream)))
        # the default ref contributes nothing, the branch still has to be walked
        assert seen == [({"project_id": 1, "ref": "H1..H2"}, frozenset({404}))]

    def test_branch_at_the_default_head_is_skipped(self):
        parent = FakeParent([({"mode": "instance"}, [self._project()])])
        stream = _stream(
            parent=parent,
            branches=FakeBranches({1: [
                {"name": "main", "commit_sha": "H1"},
                {"name": "same-as-main", "commit_sha": "H1"},
                {"name": "headless", "commit_sha": None},
            ]}),
        )
        stream.state = {"projects": {"1": {"default_head": "H1"}}}
        seen = self._capture(stream)
        assert list(stream._diff_tasks(_HeadFrontier(stream))) == []
        assert seen == []

    def test_branch_whose_head_has_not_moved_is_skipped(self):
        parent = FakeParent([({"mode": "instance"}, [self._project()])])
        stream = _stream(
            parent=parent,
            branches=FakeBranches({1: [
                {"name": "main", "commit_sha": "H1"},
                {"name": "feat", "commit_sha": "H2"},
            ]}),
        )
        stream.state = {
            "projects": {"1": {"default_head": "H1", "branches": {"feat": "H2"}}}
        }
        seen = self._capture(stream)
        assert list(stream._diff_tasks(_HeadFrontier(stream))) == []
        assert seen == []

    def test_deleted_branch_is_pruned_from_state(self):
        parent = FakeParent([({"mode": "instance"}, [self._project()])])
        stream = _stream(
            parent=parent,
            branches=FakeBranches({1: [{"name": "main", "commit_sha": "H1"}]}),
        )
        stream.state = {
            "projects": {"1": {"default_head": "H1", "branches": {"gone": "H7"}}}
        }
        self._capture(stream)
        list(stream._diff_tasks(_HeadFrontier(stream)))
        assert stream.state["projects"]["1"]["branches"] == {}

    def test_project_without_default_head_skipped(self):
        parent = FakeParent([({"mode": "instance"}, [
            self._project(), {"id": 2},  # no default_branch
        ])])
        stream = _stream(parent=parent, branches=FakeBranches({1: []}))
        self._capture(stream)
        assert list(stream._diff_tasks(_HeadFrontier(stream))) == []


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
        stream._iter_shas = lambda s, **kw: iter(["sha1"])
        fake = fake_paginate_yielding([{
            "old_path": "a.py", "new_path": "a.py", "diff": "",
        }])
        monkeypatch.setattr(concurrency, "paginate", fake)
        monkeypatch.setattr(concurrency, "imap_bounded", fake_imap_bounded)
        records = list(stream.read_records(sync_mode=None))
        assert [r["unique_key"] for r in records] == ["T:S:1:sha1:a.py"]
        # the only diff task completed → default_head advanced
        assert stream.state["projects"]["1"]["default_head"] == "H1"
