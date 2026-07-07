from __future__ import annotations

from source_gitlab.streams import concurrency
from source_gitlab.streams.commits import CommitsStream
from source_gitlab.streams.concurrency import RequestGate
from tests.conftest import (
    BASE,
    FakeBranches,
    FakeParent,
    fake_imap_stream,
    fake_walk_window_yielding,
)


def _commits(parent=None, branches=None, start_date=None) -> CommitsStream:
    return CommitsStream(
        parent=parent or FakeParent([]),
        branches=branches or FakeBranches({}),
        gate=RequestGate(1),
        start_date=start_date,
        **BASE,
    )


def _drive(gen):
    """Consume a generator, returning (yielded_items, return_value)."""
    items = []
    while True:
        try:
            items.append(next(gen))
        except StopIteration as stop:
            return items, stop.value


class TestState:
    def test_roundtrip(self):
        stream = _commits()
        assert stream.state == {}
        stream.state = None
        assert stream.state == {}
        stream._project_state(1)["default_head"] = "H1"
        assert stream.state["projects"]["1"]["default_head"] == "H1"

    def test_snapshot_defaults(self):
        stream = _commits()
        assert stream._snapshot(1) == {"default_head": None, "branches": {}}
        stream.state = {"projects": {"1": {"default_head": "H1", "branches": {"f": "H2"}}}}
        assert stream._snapshot(1) == {"default_head": "H1", "branches": {"f": "H2"}}


class TestProjectTasks:
    def test_filters_projects_without_default_branch(self):
        parent = FakeParent([({"mode": "instance"}, [
            {"id": 1, "default_branch": "main"},
            {"id": 2},                      # no default branch
            {"id": None, "default_branch": "main"},
        ])])
        tasks = list(_commits(parent=parent)._project_tasks())
        assert [t["project_id"] for t in tasks] == [1]
        assert tasks[0]["default"] == "main"
        assert tasks[0]["snapshot"] == {"default_head": None, "branches": {}}


class TestFetchProject:
    def _task(self, stream, project_id=1):
        return {
            "project": {"id": project_id, "default_branch": "main"},
            "project_id": project_id,
            "default": "main",
            "snapshot": stream._snapshot(project_id),
        }

    def test_no_default_head_returns_sentinel(self):
        stream = _commits(branches=FakeBranches({1: [{"name": "other", "commit_sha": "X"}]}))
        items, outcome = _drive(stream._fetch_project(self._task(stream)))
        assert items == []
        assert outcome == {"current_branches": None, "advances": []}

    def test_fresh_project_walks_default_and_feature(self):
        stream = _commits(branches=FakeBranches({1: [
            {"name": "main", "commit_sha": "H1"},
            {"name": "feat", "commit_sha": "H2"},
            {"name": "same-as-main", "commit_sha": "H1"},   # head == default → skip
        ]}))
        walked = []
        stream._walk_ref = lambda pid, ref, **kw: iter(walked.append((pid, ref, kw)) or [])
        items, outcome = _drive(stream._fetch_project(self._task(stream)))
        assert walked == [(1, "main", {}), (1, "H1..H2", {})]
        assert outcome["advances"] == [("default", "H1"), ("branch", "feat", "H2")]
        assert outcome["current_branches"] == {"main", "feat", "same-as-main"}

    def test_default_moved_walks_range_without_404_skip(self):
        stream = _commits(branches=FakeBranches({1: [
            {"name": "main", "commit_sha": "H9"},
        ]}))
        stream.state = {"projects": {"1": {"default_head": "H1"}}}
        walked = []
        stream._walk_ref = lambda pid, ref, **kw: iter(walked.append((pid, ref, kw)) or [])
        _, outcome = _drive(stream._fetch_project(self._task(stream)))
        assert walked == [(1, "H1..H9", {"skip_404": False})]
        assert outcome["advances"] == [("default", "H9")]

    def test_unchanged_default_and_stored_branch_skipped(self):
        stream = _commits(branches=FakeBranches({1: [
            {"name": "main", "commit_sha": "H1"},
            {"name": "feat", "commit_sha": "H2"},
        ]}))
        stream.state = {"projects": {"1": {"default_head": "H1", "branches": {"feat": "H2"}}}}
        walked = []
        stream._walk_ref = lambda pid, ref, **kw: iter(walked.append((pid, ref, kw)) or [])
        _, outcome = _drive(stream._fetch_project(self._task(stream)))
        assert walked == []
        assert outcome["advances"] == []


class TestWalkRef:
    def test_since_and_skip_flag_propagated(self, monkeypatch):
        stream = _commits(start_date="2026-06-01T00:00:00Z")
        fake = fake_walk_window_yielding([
            {"id": "sha1", "message": "m", "stats": {"additions": 1, "deletions": 2, "total": 3}},
        ])
        monkeypatch.setattr(concurrency, "walk_window", fake)
        records = list(stream._walk_ref(1, "main"))
        assert records[0]["unique_key"] == "T:S:1:sha1"
        assert records[0]["stats_total"] == 3
        assert fake.calls[0]["base_slice"]["since"] == "2026-06-01T00:00:00Z"
        assert "skip_404" not in fake.calls[0]["base_slice"]
        assert fake.calls[0]["path"] == "projects/1/repository/commits"
        assert fake.calls[0]["params"]["ref_name"] == "main"
        assert fake.calls[0]["params"]["with_stats"] == "true"

    def test_skip_404_false_included(self, monkeypatch):
        stream = _commits()
        fake = fake_walk_window_yielding([])
        monkeypatch.setattr(concurrency, "walk_window", fake)
        list(stream._walk_ref(1, "A..B", skip_404=False))
        assert fake.calls[0]["base_slice"]["skip_404"] is False


class TestApply:
    def test_sentinel_outcome_ignored(self):
        stream = _commits()
        stream._apply({"project_id": 1}, {"current_branches": None, "advances": []})
        assert stream.state == {}

    def test_advances_and_prunes_dead_branches(self):
        stream = _commits()
        stream.state = {"projects": {"1": {
            "default_head": "H0",
            "branches": {"dead": "X", "feat": "H2"},
        }}}
        stream._apply(
            {"project_id": 1},
            {"current_branches": {"main", "feat"},
             "advances": [("default", "H9"), ("branch", "feat", "H3")]},
        )
        pstate = stream.state["projects"]["1"]
        assert pstate["default_head"] == "H9"
        assert pstate["branches"] == {"feat": "H3"}  # "dead" pruned


class TestReadRecordsEndToEnd:
    def test_yields_records_then_applies_state(self, monkeypatch):
        parent = FakeParent([({"mode": "instance"}, [
            {"id": 1, "default_branch": "main"},
        ])])
        stream = _commits(
            parent=parent,
            branches=FakeBranches({1: [{"name": "main", "commit_sha": "H1"}]}),
        )
        walk_fake = fake_walk_window_yielding([{"id": "sha1", "committed_date": "2026-06-01"}])
        monkeypatch.setattr(concurrency, "walk_window", walk_fake)
        monkeypatch.setattr(concurrency, "imap_stream", fake_imap_stream)
        records = list(stream.read_records(sync_mode=None))
        assert [r["unique_key"] for r in records] == ["T:S:1:sha1"]
        assert stream.state["projects"]["1"]["default_head"] == "H1"


class TestProjection:
    def test_stream_slices_single_empty(self):
        assert list(_commits().stream_slices()) == [{}]

    def test_params_with_until(self):
        stream = _commits()
        params = stream._initial_params({"ref": "main", "since": "s", "until": "u"})
        assert params["since"] == "s"
        assert params["until"] == "u"

    def test_projection_counts_parents(self):
        stream = _commits()
        out = stream._envelope(
            {"id": "sha1", "title": "t", "message": "m",
             "parent_ids": ["a", "b"], "stats": {}},
            {"project_id": 1},
        )
        assert out["parent_count"] == 2
        assert out["stats_additions"] is None
