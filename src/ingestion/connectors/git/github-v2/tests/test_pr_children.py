"""Tests for the per-PR child streams: reviews, comments, and PR commits.

All three follow the same embedded-first pattern: records embedded in the
bulk PR query are yielded from disk, then an overflow GraphQL fetch runs
only when the embedded page was incomplete.
"""

from __future__ import annotations

import pytest
from airbyte_cdk.sources.streams.http import HttpStream
from source_github_v2.streams.base import GitHubAuthError
from source_github_v2.streams.comments import CommentsStream
from source_github_v2.streams.pr_commits import PRCommitsStream
from source_github_v2.streams.reviews import ReviewsStream
from tests.conftest import SHARED, FakePRParent, FakeResponse, graphql_body

PR_UPDATED = "2026-01-10T00:00:00Z"


def _pr(number=1, **over):
    pr = {
        "repo_owner": "acme",
        "repo_name": "r1",
        "number": number,
        "database_id": 1000 + number,
        "updated_at": PR_UPDATED,
        "commit_count": 2,
        "comment_count": 2,
        "review_count": 2,
        "embedded_offset": 0,
        "commits_complete": True,
        "commits_end_cursor": None,
        "reviews_complete": True,
        "reviews_end_cursor": None,
        "comments_complete": True,
        "comments_end_cursor": None,
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
    }
    s.update(over)
    return s


def _review_node(database_id=5, state="APPROVED"):
    return {
        "databaseId": database_id,
        "state": state,
        "body": "lgtm",
        "submittedAt": "2026-01-09T00:00:00Z",
        "authorAssociation": "MEMBER",
        "commit": {"oid": "c1"},
        "author": {"login": "al", "databaseId": 7},
    }


def _comment_node(database_id=6):
    return {
        "databaseId": database_id,
        "body": "hi",
        "createdAt": "2026-01-08T00:00:00Z",
        "updatedAt": "2026-01-09T00:00:00Z",
        "author": {"login": "al", "databaseId": 7},
        "authorAssociation": "MEMBER",
    }


def _commit_node(oid="c1"):
    return {"commit": {"oid": oid, "committedDate": "2026-01-07T00:00:00Z"}}


# ---------------------------------------------------------------------------
# Shared slicing behaviour (parametrized across the three streams)
# ---------------------------------------------------------------------------

STREAMS = [
    (ReviewsStream, "reviews_complete", "reviews_end_cursor"),
    (CommentsStream, "comments_complete", "comments_end_cursor"),
    (PRCommitsStream, "commits_complete", "commits_end_cursor"),
]


class TestSlicingShared:
    @pytest.mark.parametrize("cls,complete_key,cursor_key", STREAMS)
    def test_slice_fields(self, cls, complete_key, cursor_key):
        stream = cls(parent=FakePRParent(slices=[_pr()]), **SHARED)
        [s] = list(stream.stream_slices())
        assert s["owner"] == "acme" and s["repo"] == "r1"
        assert s["pr_number"] == 1 and s["pr_database_id"] == 1001
        assert s["partition_key"] == "acme/r1/1"
        assert s[complete_key] is True and s[cursor_key] is None

    @pytest.mark.parametrize("cls,complete_key,cursor_key", STREAMS)
    def test_unchanged_pr_skipped(self, cls, complete_key, cursor_key):
        stream = cls(parent=FakePRParent(slices=[_pr()]), **SHARED)
        state = {"acme/r1/1": {"synced_at": PR_UPDATED}}
        assert list(stream.stream_slices(stream_state=state)) == []

    @pytest.mark.parametrize("cls,complete_key,cursor_key", STREAMS)
    def test_updated_pr_resynced(self, cls, complete_key, cursor_key):
        stream = cls(parent=FakePRParent(slices=[_pr()]), **SHARED)
        state = {"acme/r1/1": {"synced_at": "2026-01-01T00:00:00Z"}}
        assert len(list(stream.stream_slices(stream_state=state))) == 1

    @pytest.mark.parametrize("cls,complete_key,cursor_key", STREAMS)
    def test_incomplete_pr_meta_skipped(self, cls, complete_key, cursor_key):
        stream = cls(parent=FakePRParent(slices=[{"repo_owner": "acme"}]), **SHARED)
        assert list(stream.stream_slices()) == []

    @pytest.mark.parametrize("cls,count_key", [(ReviewsStream, "review_count"), (CommentsStream, "comment_count")])
    def test_zero_count_pr_skipped(self, cls, count_key):
        stream = cls(parent=FakePRParent(slices=[_pr(**{count_key: 0})]), **SHARED)
        assert list(stream.stream_slices()) == []

    @pytest.mark.parametrize("cls,complete_key,cursor_key", STREAMS)
    def test_variables_with_overflow_cursor(self, cls, complete_key, cursor_key):
        stream = cls(parent=FakePRParent(), **SHARED)
        variables = stream._variables(_slice(_overflow_after="ovf"))
        assert variables["after"] == "ovf"
        assert variables["prNumber"] == 1 and variables["first"] == 100

    @pytest.mark.parametrize("cls,complete_key,cursor_key", STREAMS)
    def test_variables_page_token_wins_over_overflow(self, cls, complete_key, cursor_key):
        stream = cls(parent=FakePRParent(), **SHARED)
        variables = stream._variables(_slice(_overflow_after="ovf"), next_page_token={"after": "page2"})
        assert variables["after"] == "page2"

    @pytest.mark.parametrize("cls,complete_key,cursor_key", STREAMS)
    def test_get_updated_state_sets_synced_at(self, cls, complete_key, cursor_key):
        stream = cls(parent=FakePRParent(), **SHARED)
        record = {"repo_owner": "acme", "repo_name": "r1", "pr_number": 1, "pull_request_updated_at": PR_UPDATED}
        state = stream.get_updated_state({}, record)
        assert state == {"acme/r1/1": {"synced_at": PR_UPDATED}}

    @pytest.mark.parametrize(
        "cls,field", [(ReviewsStream, "reviews"), (CommentsStream, "comments"), (PRCommitsStream, "commits")]
    )
    def test_extract_page_info(self, cls, field):
        stream = cls(parent=FakePRParent(), **SHARED)
        body = graphql_body(["repository", "pullRequest", field], [], {"hasNextPage": True, "endCursor": "e"})
        assert stream._extract_page_info(body["data"]) == {"hasNextPage": True, "endCursor": "e"}
        assert stream._extract_page_info({"repository": {"pullRequest": None}}) == {}

    @pytest.mark.parametrize("cls,complete_key,cursor_key", STREAMS)
    def test_get_updated_state_frozen_on_errors(self, cls, complete_key, cursor_key):
        stream = cls(parent=FakePRParent(), **SHARED)
        stream._partitions_with_errors.add("acme/r1/1")
        record = {"repo_owner": "acme", "repo_name": "r1", "pr_number": 1, "pull_request_updated_at": PR_UPDATED}
        assert stream.get_updated_state({}, record) == {}


# ---------------------------------------------------------------------------
# ReviewsStream
# ---------------------------------------------------------------------------


class TestReviews:
    def test_record_mapping(self):
        stream = ReviewsStream(parent=FakePRParent(), **SHARED)
        record = stream._make_record(_review_node(), _slice())
        assert record["unique_key"] == "T:S:acme:r1:1001:5"
        assert record["state"] == "APPROVED"
        assert record["author_login"] == "al"
        assert record["commit_id"] == "c1"
        assert record["pull_request_updated_at"] == PR_UPDATED

    def test_pending_reviews_dropped(self):
        stream = ReviewsStream(parent=FakePRParent(), **SHARED)
        assert stream._make_record(_review_node(state="PENDING"), _slice()) is None

    def test_embedded_complete_no_http(self):
        parent = FakePRParent(embedded={(0, "reviews"): {"nodes": [_review_node(), _review_node(9, "PENDING")]}})
        stream = ReviewsStream(parent=parent, **SHARED)
        records = list(stream.read_records(stream_slice=_slice(reviews_complete=True)))
        assert [r["database_id"] for r in records] == [5]  # PENDING filtered

    def test_overflow_continues_from_embedded_cursor(self, monkeypatch):
        parent = FakePRParent(embedded={(0, "reviews"): {"nodes": [_review_node()]}})
        stream = ReviewsStream(parent=parent, **SHARED)
        seen_slices = []

        def fake_read(self, sync_mode=None, stream_slice=None, stream_state=None, **kw):
            seen_slices.append(stream_slice)
            yield {"database_id": 99}

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        records = list(stream.read_records(stream_slice=_slice(reviews_complete=False, reviews_end_cursor="emb-cur")))
        assert [r["database_id"] for r in records] == [5, 99]
        assert seen_slices[0]["_overflow_after"] == "emb-cur"

    def test_missing_embedded_data_full_fetch(self, monkeypatch):
        stream = ReviewsStream(parent=FakePRParent(), **SHARED)
        seen_slices = []

        def fake_read(self, sync_mode=None, stream_slice=None, stream_state=None, **kw):
            seen_slices.append(stream_slice)
            return iter(())

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        s = _slice(reviews_complete=True, reviews_end_cursor="stale")
        assert list(stream.read_records(stream_slice=s)) == []
        # Full fetch ignores the stale embedded cursor
        assert "_overflow_after" not in seen_slices[0]

    def test_auth_error_propagates(self):
        class AuthFailParent:
            def read_embedded_data(self, offset, field):
                raise GitHubAuthError("401")

        stream = ReviewsStream(parent=AuthFailParent(), **SHARED)
        with pytest.raises(GitHubAuthError):
            list(stream.read_records(stream_slice=_slice()))

    def test_other_errors_logged_and_reraised(self, caplog):
        class BrokenParent:
            def read_embedded_data(self, offset, field):
                raise ValueError("corrupt")

        stream = ReviewsStream(parent=BrokenParent(), **SHARED)
        with caplog.at_level("ERROR", logger="airbyte"), pytest.raises(ValueError):
            list(stream.read_records(stream_slice=_slice()))
        assert "Failed reviews slice acme/r1/1" in caplog.text

    def test_parse_response_overflow_page(self):
        stream = ReviewsStream(parent=FakePRParent(), **SHARED)
        body = graphql_body(["repository", "pullRequest", "reviews"], [_review_node(), _review_node(9, "PENDING")])
        records = list(stream.parse_response(FakeResponse(body), stream_slice=_slice()))
        assert [r["database_id"] for r in records] == [5]

    def test_parse_response_errors_without_data_raise(self):
        stream = ReviewsStream(parent=FakePRParent(), **SHARED)
        with pytest.raises(RuntimeError, match="GraphQL errors"):
            list(stream.parse_response(FakeResponse({"errors": [{"message": "boom"}]}), stream_slice=_slice()))

    def test_parse_response_partial_errors_freeze_partition(self):
        stream = ReviewsStream(parent=FakePRParent(), **SHARED)
        body = graphql_body(["repository", "pullRequest", "reviews"], [_review_node()])
        body["errors"] = [{"message": "partial"}]
        records = list(stream.parse_response(FakeResponse(body), stream_slice=_slice()))
        assert len(records) == 1
        assert "acme/r1/1" in stream._partitions_with_errors

    def test_schema_and_query(self):
        stream = ReviewsStream(parent=FakePRParent(), **SHARED)
        assert "reviews(first: $first" in stream._query()
        assert "state" in stream.get_json_schema()["properties"]


# ---------------------------------------------------------------------------
# CommentsStream
# ---------------------------------------------------------------------------


class TestComments:
    def test_record_mapping(self):
        stream = CommentsStream(parent=FakePRParent(), **SHARED)
        record = stream._make_record(_comment_node(), _slice())
        assert record["unique_key"] == "T:S:acme:r1:1001:6"
        assert record["body"] == "hi"
        assert record["author_id"] == 7
        assert record["pull_request_updated_at"] == PR_UPDATED

    def test_embedded_complete_no_http(self):
        parent = FakePRParent(embedded={(0, "comments"): {"nodes": [_comment_node()]}})
        stream = CommentsStream(parent=parent, **SHARED)
        records = list(stream.read_records(stream_slice=_slice(comments_complete=True)))
        assert [r["database_id"] for r in records] == [6]

    def test_overflow_continues_from_embedded_cursor(self, monkeypatch):
        parent = FakePRParent(embedded={(0, "comments"): {"nodes": [_comment_node()]}})
        stream = CommentsStream(parent=parent, **SHARED)
        seen_slices = []

        def fake_read(self, sync_mode=None, stream_slice=None, stream_state=None, **kw):
            seen_slices.append(stream_slice)
            return iter(())

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        list(stream.read_records(stream_slice=_slice(comments_complete=False, comments_end_cursor="emb-cur")))
        assert seen_slices[0]["_overflow_after"] == "emb-cur"

    def test_missing_embedded_data_full_fetch(self, monkeypatch):
        stream = CommentsStream(parent=FakePRParent(), **SHARED)
        seen_slices = []

        def fake_read(self, sync_mode=None, stream_slice=None, stream_state=None, **kw):
            seen_slices.append(stream_slice)
            return iter(())

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        s = _slice(comments_complete=True, comments_end_cursor="stale")
        assert list(stream.read_records(stream_slice=s)) == []
        assert "_overflow_after" not in seen_slices[0]

    def test_auth_error_propagates(self):
        class AuthFailParent:
            def read_embedded_data(self, offset, field):
                raise GitHubAuthError("401")

        stream = CommentsStream(parent=AuthFailParent(), **SHARED)
        with pytest.raises(GitHubAuthError):
            list(stream.read_records(stream_slice=_slice()))

    def test_errors_logged_and_reraised(self, caplog):
        class BrokenParent:
            def read_embedded_data(self, offset, field):
                raise ValueError("corrupt")

        stream = CommentsStream(parent=BrokenParent(), **SHARED)
        with caplog.at_level("ERROR", logger="airbyte"), pytest.raises(ValueError):
            list(stream.read_records(stream_slice=_slice()))
        assert "Failed comments slice acme/r1/1" in caplog.text

    def test_parse_response_overflow_page(self):
        stream = CommentsStream(parent=FakePRParent(), **SHARED)
        body = graphql_body(["repository", "pullRequest", "comments"], [_comment_node()])
        records = list(stream.parse_response(FakeResponse(body), stream_slice=_slice()))
        assert [r["database_id"] for r in records] == [6]

    def test_parse_response_errors_without_data_raise(self):
        stream = CommentsStream(parent=FakePRParent(), **SHARED)
        with pytest.raises(RuntimeError, match="GraphQL errors"):
            list(stream.parse_response(FakeResponse({"errors": [{"message": "boom"}]}), stream_slice=_slice()))

    def test_parse_response_partial_errors_freeze_partition(self):
        stream = CommentsStream(parent=FakePRParent(), **SHARED)
        body = graphql_body(["repository", "pullRequest", "comments"], [_comment_node()])
        body["errors"] = [{"message": "partial"}]
        records = list(stream.parse_response(FakeResponse(body), stream_slice=_slice()))
        assert len(records) == 1
        assert "acme/r1/1" in stream._partitions_with_errors

    def test_schema_and_query(self):
        stream = CommentsStream(parent=FakePRParent(), **SHARED)
        assert "comments(first: $first" in stream._query()
        assert "body" in stream.get_json_schema()["properties"]


# ---------------------------------------------------------------------------
# PRCommitsStream
# ---------------------------------------------------------------------------


class TestPRCommits:
    def test_embedded_complete_no_http(self):
        parent = FakePRParent(embedded={(0, "commits"): {"nodes": [_commit_node("c1"), {"commit": {}}]}})
        stream = PRCommitsStream(parent=parent, **SHARED)
        records = list(stream.read_records(stream_slice=_slice(commits_complete=True)))
        assert [r["sha"] for r in records] == ["c1"]  # sha-less node dropped
        assert records[0]["unique_key"] == "T:S:acme:r1:1001:c1"
        assert records[0]["pull_request_id"] == 1001

    def test_overflow_continues_from_embedded_cursor(self, monkeypatch):
        parent = FakePRParent(embedded={(0, "commits"): {"nodes": [_commit_node()]}})
        stream = PRCommitsStream(parent=parent, **SHARED)
        seen_slices = []

        def fake_read(self, sync_mode=None, stream_slice=None, stream_state=None, **kw):
            seen_slices.append(stream_slice)
            return iter(())

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        list(stream.read_records(stream_slice=_slice(commits_complete=False, commits_end_cursor="emb-cur")))
        assert seen_slices[0]["_overflow_after"] == "emb-cur"

    def test_missing_embedded_data_full_fetch(self, monkeypatch):
        stream = PRCommitsStream(parent=FakePRParent(), **SHARED)
        seen_slices = []

        def fake_read(self, sync_mode=None, stream_slice=None, stream_state=None, **kw):
            seen_slices.append(stream_slice)
            return iter(())

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        list(stream.read_records(stream_slice=_slice(commits_complete=True)))
        assert "_overflow_after" not in seen_slices[0]

    def test_auth_error_propagates(self):
        class AuthFailParent:
            def read_embedded_data(self, offset, field):
                raise GitHubAuthError("401")

        stream = PRCommitsStream(parent=AuthFailParent(), **SHARED)
        with pytest.raises(GitHubAuthError):
            list(stream.read_records(stream_slice=_slice()))

    def test_errors_logged_and_reraised(self, caplog):
        class BrokenParent:
            def read_embedded_data(self, offset, field):
                raise ValueError("corrupt")

        stream = PRCommitsStream(parent=BrokenParent(), **SHARED)
        with caplog.at_level("ERROR", logger="airbyte"), pytest.raises(ValueError):
            list(stream.read_records(stream_slice=_slice()))
        assert "Failed pr_commits slice acme/r1/1" in caplog.text

    def test_parse_response_overflow_page(self):
        stream = PRCommitsStream(parent=FakePRParent(), **SHARED)
        body = graphql_body(["repository", "pullRequest", "commits"], [_commit_node("c9"), {"commit": None}])
        records = list(stream.parse_response(FakeResponse(body), stream_slice=_slice()))
        assert [r["sha"] for r in records] == ["c9"]

    def test_parse_response_errors_without_data_raise(self):
        stream = PRCommitsStream(parent=FakePRParent(), **SHARED)
        with pytest.raises(RuntimeError, match="GraphQL errors"):
            list(stream.parse_response(FakeResponse({"errors": [{"message": "boom"}]}), stream_slice=_slice()))

    def test_parse_response_partial_errors_freeze_partition(self):
        stream = PRCommitsStream(parent=FakePRParent(), **SHARED)
        body = graphql_body(["repository", "pullRequest", "commits"], [_commit_node()])
        body["errors"] = [{"message": "partial"}]
        records = list(stream.parse_response(FakeResponse(body), stream_slice=_slice()))
        assert len(records) == 1
        assert "acme/r1/1" in stream._partitions_with_errors

    def test_schema_and_query(self):
        stream = PRCommitsStream(parent=FakePRParent(), **SHARED)
        assert "commits(first: $first" in stream._query()
        assert "sha" in stream.get_json_schema()["properties"]
