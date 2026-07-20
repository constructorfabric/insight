from __future__ import annotations

import pytest
from airbyte_cdk.sources.streams.http import HttpStream
from source_github_v2.streams.base import GitHubAuthError
from source_github_v2.streams.review_comments import ReviewCommentsStream
from tests.conftest import SHARED, FakePRParent, FakeResponse, graphql_body

RT_PATH = ["repository", "pullRequest", "reviewThreads"]
PR_UPDATED = "2026-01-10T00:00:00Z"


def _stream(parent=None) -> ReviewCommentsStream:
    return ReviewCommentsStream(parent=parent or FakePRParent(), **SHARED)


def _pr(number=1, **over):
    pr = {
        "repo_owner": "acme",
        "repo_name": "r1",
        "number": number,
        "database_id": 1000 + number,
        "updated_at": PR_UPDATED,
        "review_count": 2,
        "embedded_offset": 0,
        "review_threads_has_next_page": False,
        "review_threads_end_cursor": None,
    }
    pr.update(over)
    return pr


def _slice(number=1, **over):
    s = {
        "owner": "acme",
        "repo": "r1",
        "pr_number": number,
        "pr_database_id": 1000 + number,
        "pr_updated_at": PR_UPDATED,
        "partition_key": f"acme/r1/{number}",
        "embedded_offset": 0,
        "review_threads_has_next_page": False,
        "review_threads_end_cursor": None,
    }
    s.update(over)
    return s


def _comment_node(database_id=5):
    return {
        "databaseId": database_id,
        "body": "nit",
        "path": "src/a.py",
        "line": 10,
        "startLine": 8,
        "diffHunk": "@@",
        "createdAt": "2026-01-08T00:00:00Z",
        "updatedAt": "2026-01-09T00:00:00Z",
        "author": {"login": "al", "databaseId": 7},
        "authorAssociation": "MEMBER",
        "commit": {"oid": "c1"},
        "originalCommit": {"oid": "c0"},
        "replyTo": {"databaseId": 4},
    }


def _thread(comments, thread_id="T1", resolved=True, has_next=False, end_cursor=None):
    return {
        "id": thread_id,
        "isResolved": resolved,
        "comments": {"nodes": comments, "pageInfo": {"hasNextPage": has_next, "endCursor": end_cursor}},
    }


class TestSlicing:
    def test_slice_fields(self):
        stream = _stream(FakePRParent(slices=[_pr()]))
        [s] = list(stream.stream_slices())
        assert s["partition_key"] == "acme/r1/1"
        assert s["review_threads_has_next_page"] is False

    def test_zero_reviews_skipped(self):
        stream = _stream(FakePRParent(slices=[_pr(review_count=0)]))
        assert list(stream.stream_slices()) == []

    def test_unchanged_pr_skipped(self):
        stream = _stream(FakePRParent(slices=[_pr()]))
        state = {"acme/r1/1": {"synced_at": PR_UPDATED}}
        assert list(stream.stream_slices(stream_state=state)) == []

    def test_incomplete_pr_meta_skipped(self):
        stream = _stream(FakePRParent(slices=[{"repo_owner": "acme"}]))
        assert list(stream.stream_slices()) == []

    def test_variables_with_overflow_cursor(self):
        variables = _stream()._variables(_slice(_overflow_after="ovf"))
        assert variables["after"] == "ovf"

    def test_variables_page_token_wins_over_overflow(self):
        variables = _stream()._variables(_slice(_overflow_after="ovf"), next_page_token={"after": "page2"})
        assert variables["after"] == "page2"

    def test_extract_page_info(self):
        body = graphql_body(RT_PATH, [], {"hasNextPage": True, "endCursor": "e"})
        assert _stream()._extract_page_info(body["data"]) == {"hasNextPage": True, "endCursor": "e"}
        assert _stream()._extract_page_info({"repository": None}) == {}


class TestMakeRecord:
    def test_record_mapping(self):
        record = _stream()._make_record(_comment_node(), {"isResolved": True}, _slice())
        assert record["unique_key"] == "T:S:acme:r1:1001:rc:5"
        assert record["filename"] == "src/a.py"
        assert record["line"] == 10 and record["start_line"] == 8
        assert record["commit_id"] == "c1"
        assert record["original_commit_id"] == "c0"
        assert record["in_reply_to_id"] == 4
        assert record["thread_resolved"] is True
        assert record["pull_request_updated_at"] == PR_UPDATED

    def test_nullable_relations_tolerated(self):
        node = _comment_node()
        node.update({"commit": None, "originalCommit": None, "replyTo": None, "author": None})
        record = _stream()._make_record(node, {"isResolved": False}, _slice())
        assert record["commit_id"] is None
        assert record["in_reply_to_id"] is None
        assert record["author_login"] is None


class TestYieldThreadComments:
    def test_embedded_comments_only(self):
        records = list(_stream()._yield_thread_comments(_thread([_comment_node()]), _slice()))
        assert [r["database_id"] for r in records] == [5]

    def test_thread_without_id_cannot_overflow(self):
        thread = _thread([_comment_node()], has_next=True, end_cursor="c")
        thread["id"] = None
        records = list(_stream()._yield_thread_comments(thread, _slice()))
        assert len(records) == 1

    def test_comment_overflow_paginates(self, monkeypatch):
        stream = _stream()
        calls = []
        pages = [
            {
                "data": {
                    "node": {
                        "isResolved": False,
                        "comments": {"nodes": [_comment_node(6)], "pageInfo": {"hasNextPage": True, "endCursor": "p2"}},
                    }
                }
            },
            {
                "data": {
                    "node": {
                        "isResolved": False,
                        "comments": {
                            "nodes": [_comment_node(7)],
                            "pageInfo": {"hasNextPage": False, "endCursor": None},
                        },
                    }
                }
            },
        ]

        def fake_send(query, variables, max_retries=5):
            calls.append(variables)
            return pages.pop(0)

        monkeypatch.setattr(stream, "_send_graphql", fake_send)
        thread = _thread([_comment_node(5)], thread_id="T1", has_next=True, end_cursor="p1")
        records = list(stream._yield_thread_comments(thread, _slice()))
        assert [r["database_id"] for r in records] == [5, 6, 7]
        # Overflow comments carry the thread's resolved flag from the overflow page
        assert records[1]["thread_resolved"] is False
        assert [c["after"] for c in calls] == ["p1", "p2"]
        assert calls[0]["threadId"] == "T1"

    def test_overflow_errors_freeze_partition(self, monkeypatch):
        stream = _stream()
        body = {
            "errors": [{"message": "partial"}],
            "data": {"node": {"isResolved": True, "comments": {"nodes": [], "pageInfo": {"hasNextPage": False}}}},
        }
        monkeypatch.setattr(stream, "_send_graphql", lambda *a, **kw: body)
        thread = _thread([], has_next=True, end_cursor="p1")
        list(stream._yield_thread_comments(thread, _slice()))
        assert "acme/r1/1" in stream._partitions_with_errors


class TestParseResponse:
    def test_thread_pages_flattened(self):
        body = graphql_body(RT_PATH, [_thread([_comment_node(5)]), _thread([_comment_node(6)])])
        records = list(_stream().parse_response(FakeResponse(body), stream_slice=_slice()))
        assert [r["database_id"] for r in records] == [5, 6]

    def test_errors_without_data_raise(self):
        with pytest.raises(RuntimeError, match="GraphQL errors"):
            list(_stream().parse_response(FakeResponse({"errors": [{"message": "boom"}]}), stream_slice=_slice()))

    def test_partial_errors_freeze_partition(self):
        stream = _stream()
        body = graphql_body(RT_PATH, [_thread([_comment_node()])])
        body["errors"] = [{"message": "partial"}]
        records = list(stream.parse_response(FakeResponse(body), stream_slice=_slice()))
        assert len(records) == 1
        assert "acme/r1/1" in stream._partitions_with_errors


class TestReadRecords:
    def test_embedded_complete_no_http(self):
        parent = FakePRParent(embedded={(0, "review_threads"): {"nodes": [_thread([_comment_node()])]}})
        stream = _stream(parent)
        records = list(stream.read_records(stream_slice=_slice()))
        assert [r["database_id"] for r in records] == [5]

    def test_zero_comment_pr_marks_synced(self):
        # PRs with reviews but no inline comments must not be re-fetched forever.
        parent = FakePRParent(embedded={(0, "review_threads"): {"nodes": []}})
        stream = _stream(parent)
        assert list(stream.read_records(stream_slice=_slice())) == []
        assert stream._deferred_state_updates == {"acme/r1/1": PR_UPDATED}

    def test_thread_overflow_continues_from_cursor(self, monkeypatch):
        parent = FakePRParent(embedded={(0, "review_threads"): {"nodes": [_thread([_comment_node()])]}})
        stream = _stream(parent)
        seen_slices = []

        def fake_read(self, sync_mode=None, stream_slice=None, stream_state=None, **kw):
            seen_slices.append(stream_slice)
            return iter(())

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        list(
            stream.read_records(
                stream_slice=_slice(review_threads_has_next_page=True, review_threads_end_cursor="rt-cur")
            )
        )
        assert seen_slices[0]["_overflow_after"] == "rt-cur"

    def test_missing_embedded_data_full_fetch(self, monkeypatch):
        stream = _stream(FakePRParent())
        seen_slices = []

        def fake_read(self, sync_mode=None, stream_slice=None, stream_state=None, **kw):
            seen_slices.append(stream_slice)
            return iter(())

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        list(stream.read_records(stream_slice=_slice(review_threads_end_cursor="stale")))
        assert "_overflow_after" not in seen_slices[0]

    def test_auth_error_propagates(self):
        class AuthFailParent:
            def read_embedded_data(self, offset, field):
                raise GitHubAuthError("401")

        stream = _stream(AuthFailParent())
        with pytest.raises(GitHubAuthError):
            list(stream.read_records(stream_slice=_slice()))

    def test_other_errors_logged_and_reraised(self, caplog):
        class BrokenParent:
            def read_embedded_data(self, offset, field):
                raise ValueError("corrupt")

        stream = _stream(BrokenParent())
        with caplog.at_level("ERROR", logger="airbyte"), pytest.raises(ValueError):
            list(stream.read_records(stream_slice=_slice()))
        assert "Failed review_comments slice acme/r1/1" in caplog.text


class TestGetUpdatedState:
    def _record(self):
        return {"repo_owner": "acme", "repo_name": "r1", "pr_number": 1, "pull_request_updated_at": PR_UPDATED}

    def test_synced_at_set(self):
        state = _stream().get_updated_state({}, self._record())
        assert state == {"acme/r1/1": {"synced_at": PR_UPDATED}}

    def test_frozen_partition_untouched(self):
        stream = _stream()
        stream._partitions_with_errors.add("acme/r1/1")
        assert stream.get_updated_state({}, self._record()) == {}

    def test_deferred_updates_do_not_override_existing(self):
        stream = _stream()
        stream._deferred_state_updates = {"acme/r1/2": "2026-01-03T00:00:00Z", "acme/r1/1": "2026-01-01T00:00:00Z"}
        state = stream.get_updated_state({}, self._record())
        assert state["acme/r1/2"] == {"synced_at": "2026-01-03T00:00:00Z"}
        # Record-driven entry wins over the deferred one
        assert state["acme/r1/1"] == {"synced_at": PR_UPDATED}
        assert stream._deferred_state_updates == {}


class TestSchemaAndQuery:
    def test_query_is_review_threads_query(self):
        assert "reviewThreads(first: $first" in _stream()._query()

    def test_schema_has_inline_comment_fields(self):
        props = _stream().get_json_schema()["properties"]
        for field in ("filename", "line", "diff_hunk", "thread_resolved", "in_reply_to_id"):
            assert field in props
