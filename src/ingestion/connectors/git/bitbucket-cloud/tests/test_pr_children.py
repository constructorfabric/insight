"""Tests for the two PR child streams: pull_request_comments / pull_request_commits."""

from __future__ import annotations

from tests.conftest import FakeParent, FakeResponse


def _pr(pr_id=42, comment_count=3, updated_on="2026-06-30T00:00:00+00:00"):
    return {
        "workspace": "ws", "repo_slug": "repo", "id": pr_id,
        "comment_count": comment_count, "updated_on": updated_on,
    }


SLICE = {"parent": _pr()}


# ---------------------------------------------------------------------------
# pull_request_comments
# ---------------------------------------------------------------------------


class TestPRCommentsPath:
    def test_path(self, pr_comments_stream):
        assert pr_comments_stream._path(SLICE) == "repositories/ws/repo/pullrequests/42/comments"


class TestPRCommentsSlices:
    def test_skips_zero_comment_and_unchanged_prs(self, pr_comments_stream):
        pr_comments_stream.parent = FakeParent(records=[
            _pr(pr_id=1),                                        # to fetch
            _pr(pr_id=2, comment_count=0),                       # zero comments
            _pr(pr_id=3, updated_on="2026-06-01T00:00:00+00:00"),  # unchanged
            "junk",
        ])
        state = {"ws/repo/3": {"pull_request_updated_on": "2026-06-15T00:00:00+00:00"}}
        slices = list(pr_comments_stream.stream_slices(sync_mode=None, stream_state=state))
        assert [s["parent"]["id"] for s in slices] == [1]

    def test_updated_pr_refetched(self, pr_comments_stream):
        pr_comments_stream.parent = FakeParent(records=[
            _pr(pr_id=3, updated_on="2026-06-30T00:00:00+00:00"),
        ])
        state = {"ws/repo/3": {"pull_request_updated_on": "2026-06-15T00:00:00+00:00"}}
        slices = list(pr_comments_stream.stream_slices(sync_mode=None, stream_state=state))
        assert len(slices) == 1


class TestPRCommentsParse:
    def test_emits_comment_with_inline_context(self, pr_comments_stream):
        payload = {"values": [{
            "id": 7,
            "user": {"display_name": "Rev", "uuid": "{u-2}"},
            "content": {"raw": "looks good"},
            "created_on": "2026-06-29T00:00:00+00:00",
            "updated_on": "2026-06-29T01:00:00+00:00",
            "inline": {"path": "a.py", "from": 1, "to": 2},
            "parent": {"id": 5},
            "deleted": False,
        }]}
        records = list(pr_comments_stream.parse_response(FakeResponse(payload), stream_slice=SLICE))
        assert len(records) == 1
        rec = records[0]
        assert rec["unique_key"] == "T:S:ws:repo:42:7"
        assert rec["is_inline"] is True
        assert rec["inline_path"] == "a.py"
        assert rec["parent_comment_id"] == 5
        assert rec["pull_request_updated_on"] == "2026-06-30T00:00:00+00:00"

    def test_top_level_comment_defaults(self, pr_comments_stream):
        payload = {"values": [{"id": 8, "content": {"raw": "top"}}]}
        rec = next(iter(pr_comments_stream.parse_response(FakeResponse(payload), stream_slice=SLICE)))
        assert rec["is_inline"] is False
        assert rec["inline_path"] is None
        assert rec["parent_comment_id"] is None
        assert rec["is_deleted"] is False

    def test_comment_without_id_skipped(self, pr_comments_stream):
        payload = {"values": [{"content": {"raw": "anon"}}]}
        assert list(pr_comments_stream.parse_response(FakeResponse(payload), stream_slice=SLICE)) == []

    def test_body_truncated(self, pr_comments_stream):
        payload = {"values": [{"id": 9, "content": {"raw": "x" * 5000}}]}
        rec = next(iter(pr_comments_stream.parse_response(FakeResponse(payload), stream_slice=SLICE)))
        assert len(rec["body"].encode()) <= 1024

    def test_schema_covers_all_record_fields(self, pr_comments_stream):
        payload = {"values": [{"id": 7, "content": {"raw": "hi"}}]}
        record = next(iter(pr_comments_stream.parse_response(FakeResponse(payload), stream_slice=SLICE)))
        schema_props = set(pr_comments_stream.get_json_schema()["properties"])
        assert set(record) <= schema_props


class TestPRCommentsState:
    def test_marks_pr_synced(self, pr_comments_stream):
        record = {
            "workspace": "ws", "repo_slug": "repo", "pr_id": 42,
            "pull_request_updated_on": "2026-06-30T00:00:00+00:00",
        }
        state = pr_comments_stream.get_updated_state({}, record)
        assert state == {"ws/repo/42": {"pull_request_updated_on": "2026-06-30T00:00:00+00:00"}}

    def test_incomplete_record_ignored(self, pr_comments_stream):
        assert pr_comments_stream.get_updated_state({}, {"pr_id": 42}) == {}
        assert pr_comments_stream.get_updated_state(
            {}, {"workspace": "ws", "repo_slug": "repo"},
        ) == {}


# ---------------------------------------------------------------------------
# pull_request_commits
# ---------------------------------------------------------------------------


class TestPRCommitsPath:
    def test_path(self, pr_commits_stream):
        assert pr_commits_stream._path(SLICE) == "repositories/ws/repo/pullrequests/42/commits"


class TestPRCommitsSlices:
    def test_skips_unchanged_prs_only(self, pr_commits_stream):
        # No zero-comment skip here — commits exist regardless of comments.
        pr_commits_stream.parent = FakeParent(records=[
            _pr(pr_id=1, comment_count=0),
            _pr(pr_id=3, updated_on="2026-06-01T00:00:00+00:00"),
            "junk",
        ])
        state = {"ws/repo/3": {"pull_request_updated_on": "2026-06-15T00:00:00+00:00"}}
        slices = list(pr_commits_stream.stream_slices(sync_mode=None, stream_state=state))
        assert [s["parent"]["id"] for s in slices] == [1]


class TestPRCommitsParse:
    def test_emits_hash_linkage_only(self, pr_commits_stream):
        payload = {"values": [{
            "hash": "f" * 40,
            "author": {"user": {"uuid": "{a-1}"}},
        }]}
        records = list(pr_commits_stream.parse_response(FakeResponse(payload), stream_slice=SLICE))
        assert len(records) == 1
        rec = records[0]
        assert rec["unique_key"] == f"T:S:ws:repo:42:{'f' * 40}"
        assert rec["pr_id"] == 42
        assert rec["author_uuid"] == "{a-1}"

    def test_commit_without_hash_skipped(self, pr_commits_stream):
        payload = {"values": [{"author": {}}]}
        assert list(pr_commits_stream.parse_response(FakeResponse(payload), stream_slice=SLICE)) == []

    def test_schema_covers_all_record_fields(self, pr_commits_stream):
        payload = {"values": [{"hash": "f" * 40}]}
        record = next(iter(pr_commits_stream.parse_response(FakeResponse(payload), stream_slice=SLICE)))
        schema_props = set(pr_commits_stream.get_json_schema()["properties"])
        assert set(record) <= schema_props


class TestPRCommitsState:
    def test_marks_pr_synced(self, pr_commits_stream):
        record = {
            "workspace": "ws", "repo_slug": "repo", "pr_id": 42,
            "pull_request_updated_on": "2026-06-30T00:00:00+00:00",
        }
        state = pr_commits_stream.get_updated_state({}, record)
        assert state == {"ws/repo/42": {"pull_request_updated_on": "2026-06-30T00:00:00+00:00"}}

    def test_incomplete_record_ignored(self, pr_commits_stream):
        assert pr_commits_stream.get_updated_state({}, {"pr_id": 42}) == {}
