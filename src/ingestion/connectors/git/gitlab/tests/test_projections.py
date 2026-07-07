"""Per-stream projection tests: _path / _initial_params / _record_key / _project.

These are pure functions over API payloads — the bulk of each stream module.
"""

from __future__ import annotations

import json

import pytest

from source_gitlab.streams.base import MAX_TITLE_CHARS
from source_gitlab.streams.branches import BranchesStream
from source_gitlab.streams.concurrency import RequestGate
from source_gitlab.streams.issues import IssuesStream
from source_gitlab.streams.merge_request_approvals import MergeRequestApprovalsStream
from source_gitlab.streams.merge_request_commits import MergeRequestCommitsStream
from source_gitlab.streams.merge_request_discussions import (
    MergeRequestDiscussionsStream,
)
from source_gitlab.streams.merge_request_notes import MergeRequestNotesStream
from source_gitlab.streams.merge_request_state_events import (
    MergeRequestStateEventsStream,
)
from source_gitlab.streams.merge_requests import MergeRequestsStream
from source_gitlab.streams.projects import ProjectsStream
from source_gitlab.streams.users import UsersStream
from tests.conftest import BASE, FakeParent

SCOPED = {**BASE, "groups": (), "projects": ()}


def _child(cls):
    return cls(
        parent=FakeParent([]), gate=RequestGate(1), groups=(), projects=(),
        start_date=None, **BASE,
    )


MR_SLICE = {"project_id": 1, "mr_iid": 2, "mr_updated_at": "2026-06-01T00:00:00Z"}


# ---------------------------------------------------------------------------
# projects
# ---------------------------------------------------------------------------


class TestProjects:
    def test_paths_per_mode(self):
        stream = ProjectsStream(**SCOPED)
        assert stream._path(stream_slice={"mode": "group", "group": "a/b"}) == (
            "groups/a%2Fb/projects"
        )
        assert stream._path(stream_slice={"mode": "project", "project": "a/b"}) == (
            "projects/a%2Fb"
        )
        assert stream._path(stream_slice=None) == "projects"

    def test_params_per_mode(self):
        stream = ProjectsStream(**SCOPED)
        assert stream._initial_params({"mode": "project", "project": "x"}) == {
            "statistics": "true",
        }
        group = stream._initial_params({"mode": "group", "group": "g"})
        assert group["include_subgroups"] == "true"
        instance = stream._initial_params(None)
        assert instance["pagination"] == "keyset"
        assert instance["order_by"] == "id"

    def test_projection_and_key(self):
        stream = ProjectsStream(**SCOPED)
        record = {
            "id": 7, "name": "app", "path": "app", "path_with_namespace": "g/app",
            "namespace": {"id": 3, "full_path": "g"},
            "statistics": {"commit_count": 10, "repository_size": 2048},
            "default_branch": "main", "archived": False,
        }
        out = stream._envelope(record, None)
        assert out["unique_key"] == "T:S:7"
        assert out["namespace_full_path"] == "g"
        assert out["statistics_commit_count"] == 10
        assert out["data_source"] == "insight_gitlab"

    def test_scoped_stream_slices(self):
        unscoped = ProjectsStream(**SCOPED)
        assert list(unscoped.stream_slices()) == [{"mode": "instance"}]
        scoped = ProjectsStream(**{**BASE, "groups": ("g",), "projects": ("p",)})
        assert list(scoped.stream_slices()) == [
            {"mode": "group", "group": "g"},
            {"mode": "project", "project": "p"},
        ]


# ---------------------------------------------------------------------------
# users
# ---------------------------------------------------------------------------


class TestUsers:
    def test_paths_per_mode(self):
        stream = UsersStream(**SCOPED)
        assert stream._path(stream_slice={"mode": "group", "group": "g"}) == (
            "groups/g/members/all"
        )
        assert stream._path(stream_slice={"mode": "project", "project": "p"}) == (
            "projects/p/members/all"
        )
        assert stream._path(stream_slice=None) == "users"

    def test_params(self):
        stream = UsersStream(**SCOPED)
        assert stream._initial_params(None)["pagination"] == "keyset"
        assert stream._initial_params({"mode": "group", "group": "g"}) == {
            "per_page": stream.page_size,
        }

    def test_projection(self):
        stream = UsersStream(**SCOPED)
        out = stream._envelope({"id": 4, "username": "u", "bot": False}, None)
        assert out["unique_key"] == "T:S:4"
        assert out["username"] == "u"


# ---------------------------------------------------------------------------
# branches (projection only; transport covered in test_branches_stream)
# ---------------------------------------------------------------------------


class TestBranchesProjection:
    def _stream(self):
        return BranchesStream(parent=FakeParent([]), gate=RequestGate(1), **BASE)

    def test_path_from_parent(self):
        assert self._stream()._path(stream_slice={"parent": {"id": 5}}) == (
            "projects/5/repository/branches"
        )

    def test_path_missing_parent_field_raises_routing_error(self):
        from source_gitlab.streams.errors import GitlabApiError

        with pytest.raises(GitlabApiError, match="routing"):
            self._stream()._path(stream_slice={"parent": {}})

    def test_projection(self):
        out = self._stream()._envelope(
            {"name": "main", "commit": {"id": "sha1"}, "default": True,
             "protected": True, "merged": False},
            {"parent": {"id": 5}},
        )
        assert out["unique_key"] == "T:S:5:main"
        assert out["commit_sha"] == "sha1"
        assert out["default"] is True


# ---------------------------------------------------------------------------
# issues
# ---------------------------------------------------------------------------


class TestIssues:
    def test_projection_full(self):
        stream = _child(IssuesStream)
        record = {
            "project_id": 1, "iid": 2, "id": 3, "title": "t", "description": "d",
            "state": "closed", "author": {"id": 9, "username": "a"},
            "closed_by": {"id": 11}, "milestone": {"id": 13},
            "assignees": [{"id": 21}, {"id": 22}], "labels": ["bug"],
            "user_notes_count": 4,
            "created_at": "2026-01-01", "updated_at": "2026-01-02",
            "closed_at": "2026-01-03",
        }
        out = stream._envelope(record, None)
        assert out["unique_key"] == "T:S:1:2"
        assert out["author_username"] == "a"
        assert out["closed_by_id"] == 11
        assert json.loads(out["assignee_ids"]) == [21, 22]
        assert json.loads(out["labels"]) == ["bug"]
        assert out["title_truncated"] is False

    def test_title_truncation_flagged(self):
        stream = _child(IssuesStream)
        record = {"project_id": 1, "iid": 2, "title": "x" * (MAX_TITLE_CHARS + 5)}
        out = stream._envelope(record, None)
        assert len(out["title"]) == MAX_TITLE_CHARS
        assert out["title_truncated"] is True


# ---------------------------------------------------------------------------
# merge_requests
# ---------------------------------------------------------------------------


class TestMergeRequests:
    def test_projection_full(self):
        stream = _child(MergeRequestsStream)
        record = {
            "project_id": 1, "iid": 2, "id": 3, "title": "t", "description": None,
            "state": "merged", "draft": False,
            "author": {"id": 9, "username": "a"},
            "merged_by": {"id": 10, "username": "m"},
            "milestone": {"id": 13},
            "assignees": [{"id": 21}], "reviewers": [{"id": 31}],
            "source_branch": "f", "target_branch": "main",
            "merged_at": "2026-01-05", "sha": "s1", "merge_commit_sha": "s2",
            "squash": False, "merge_status": "can_be_merged",
            "labels": ["x"], "user_notes_count": 1,
        }
        out = stream._envelope(record, None)
        assert out["unique_key"] == "T:S:1:2"
        assert out["merged_by_username"] == "m"
        assert json.loads(out["reviewer_ids"]) == [31]
        assert out["description"] is None
        assert out["description_truncated"] is False


# ---------------------------------------------------------------------------
# MR child streams
# ---------------------------------------------------------------------------


class TestMergeRequestNotes:
    def test_path_and_projection(self):
        stream = _child(MergeRequestNotesStream)
        assert stream._path(stream_slice=MR_SLICE) == "projects/1/merge_requests/2/notes"
        record = {
            "id": 44, "body": "note", "author": {"id": 9, "username": "a"},
            "system": False, "resolvable": True, "resolved": True,
            "resolved_by": {"id": 10}, "noteable_type": "MergeRequest",
            "position": {"new_path": "a.py", "new_line": 3},
            "created_at": "2026-01-01", "updated_at": "2026-01-02",
        }
        out = stream._envelope(record, MR_SLICE)
        assert out["unique_key"] == "T:S:1:44"
        assert out["mr_iid"] == 2
        assert out["resolved_by_id"] == 10
        assert out["position_new_path"] == "a.py"
        assert out["mr_updated_at"] == "2026-06-01T00:00:00Z"


class TestMergeRequestDiscussions:
    def test_path_and_projection(self):
        stream = _child(MergeRequestDiscussionsStream)
        assert stream._path(stream_slice=MR_SLICE) == (
            "projects/1/merge_requests/2/discussions"
        )
        record = {"id": "abc", "individual_note": False,
                  "notes": [{"id": 1}, {"id": 2}]}
        out = stream._envelope(record, MR_SLICE)
        assert out["unique_key"] == "T:S:1:2:abc"
        assert out["discussion_id"] == "abc"
        assert json.loads(out["note_ids"]) == [1, 2]


class TestMergeRequestStateEvents:
    def test_path_and_projection(self):
        stream = _child(MergeRequestStateEventsStream)
        assert stream._path(stream_slice=MR_SLICE) == (
            "projects/1/merge_requests/2/resource_state_events"
        )
        record = {"id": 55, "user": {"id": 9, "username": "a"},
                  "state": "merged", "created_at": "2026-01-01"}
        out = stream._envelope(record, MR_SLICE)
        assert out["unique_key"] == "T:S:1:55"
        assert out["state"] == "merged"
        assert out["user_username"] == "a"


class TestMergeRequestCommits:
    def test_path_and_projection(self):
        stream = _child(MergeRequestCommitsStream)
        assert stream._path(stream_slice=MR_SLICE) == (
            "projects/1/merge_requests/2/commits"
        )
        record = {
            "id": "sha1", "short_id": "sha", "title": "t", "message": "m",
            "author_name": "A", "author_email": "a@x", "authored_date": "2026-01-01",
            "committer_name": "C", "committer_email": "c@x",
            "committed_date": "2026-01-02",
        }
        out = stream._envelope(record, MR_SLICE)
        assert out["unique_key"] == "T:S:1:2:sha1"
        assert out["author_email"] == "a@x"
        assert out["message_truncated"] is False


class TestMergeRequestApprovals:
    def test_path_and_projection(self):
        stream = _child(MergeRequestApprovalsStream)
        assert stream._path(stream_slice=MR_SLICE) == (
            "projects/1/merge_requests/2/approvals"
        )
        record = {
            "approvals_required": 2, "approvals_left": 1, "approved": False,
            "approved_by": [{"user": {"id": 9, "username": "a"}}, {"user": None}],
        }
        out = stream._envelope(record, MR_SLICE)
        assert out["unique_key"] == "T:S:1:2"
        approved_by = json.loads(out["approved_by"])
        assert approved_by == [
            {"id": 9, "username": "a"}, {"id": None, "username": None},
        ]

    def test_402_additionally_skippable(self):
        # Approvals API returns 402 on tiers without the feature.
        assert MergeRequestApprovalsStream.skippable_statuses == frozenset({402, 404})
