from __future__ import annotations

import pytest
from airbyte_cdk.sources.streams.http import HttpStream
from source_github_v2.streams.pull_requests import PullRequestsStream
from tests.conftest import SHARED, FakeRepoParent, FakeResponse, graphql_body

PR_PATH = ["repository", "pullRequests"]

REPO_RECORD = {"owner": "acme", "name": "r1", "default_branch": "main", "pushed_at": "2026-01-01T00:00:00Z"}
SLICE = {"owner": "acme", "repo": "r1", "partition_key": "acme/r1", "cursor_value": None}


def _stream(parent_records=(), start_date=None, **kwargs) -> PullRequestsStream:
    return PullRequestsStream(parent=FakeRepoParent(parent_records), start_date=start_date, **kwargs, **SHARED)


def _pr_node(number=1, updated="2026-01-10T00:00:00Z", state="OPEN", merged=False, **over):
    node = {
        "databaseId": 1000 + number,
        "number": number,
        "title": f"PR {number}",
        "body": "text",
        "state": state,
        "merged": merged,
        "isDraft": False,
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": updated,
        "closedAt": None,
        "mergedAt": None,
        "headRefName": "feat",
        "baseRefName": "main",
        "additions": 5,
        "deletions": 3,
        "changedFiles": 2,
        "author": {"login": "al", "databaseId": 7, "email": "al@x"},
        "reviewDecision": "APPROVED",
        "labels": {"nodes": [{"name": "bug"}, {"name": None}]},
        "milestone": {"title": "v1"},
        "mergeCommit": {"oid": "mc1"},
        "mergedBy": None,
        "commits": {
            "totalCount": 1,
            "pageInfo": {"hasNextPage": False, "endCursor": None},
            "nodes": [{"commit": {"oid": "c1", "committedDate": "2026-01-02T00:00:00Z"}}],
        },
        "comments": {"totalCount": 0, "pageInfo": {"hasNextPage": False, "endCursor": None}, "nodes": []},
        "reviews": {"totalCount": 2, "pageInfo": {"hasNextPage": True, "endCursor": "rev-cur"}, "nodes": []},
        "reviewThreads": {"pageInfo": {"hasNextPage": False, "endCursor": None}, "nodes": []},
        "reviewRequests": {
            "nodes": [
                {"requestedReviewer": {"login": "rev1", "databaseId": 9}},
                {"requestedReviewer": {"name": "Team X", "slug": "team-x"}},
                {"requestedReviewer": None},
            ]
        },
    }
    node.update(over)
    return node


class TestQueryBuilding:
    def test_default_embedded_page_sizes(self):
        query = _stream()._query()
        assert "commits(first: 10)" in query
        assert "reviews(first: 10)" in query
        assert "comments(first: 10)" in query
        assert "reviewThreads(first: 15)" in query
        assert "comments(first: 2)" in query  # thread comments

    def test_configured_embedded_page_sizes(self):
        query = _stream(
            embedded_page_sizes={"commits": 3, "reviews": 4, "comments": 5, "review_threads": 6, "thread_comments": 7}
        )._query()
        assert "commits(first: 3)" in query
        assert "reviews(first: 4)" in query
        assert "comments(first: 5)" in query
        assert "reviewThreads(first: 6)" in query
        assert "comments(first: 7)" in query
        assert "__" not in query  # no placeholder left behind


class TestVariables:
    def test_full_slice(self):
        variables = _stream(page_size=25)._variables(SLICE)
        assert variables == {
            "owner": "acme",
            "repo": "r1",
            "first": 25,
            "orderBy": {"field": "UPDATED_AT", "direction": "DESC"},
        }

    def test_incomplete_slice_raises(self):
        with pytest.raises(ValueError, match="incomplete slice"):
            _stream()._variables({"owner": "acme"})

    def test_pagination_cursor_added(self):
        variables = _stream()._variables(SLICE, next_page_token={"after": "c2"})
        assert variables["after"] == "c2"


class TestSlicing:
    def test_one_slice_per_repo_with_cursor(self):
        stream = _stream([REPO_RECORD])
        state = {"acme/r1": {"updated_at": "2026-01-05T00:00:00Z"}}
        slices = list(stream.stream_slices(stream_state=state))
        assert slices == [
            {"owner": "acme", "repo": "r1", "partition_key": "acme/r1", "cursor_value": "2026-01-05T00:00:00Z"}
        ]

    def test_incomplete_parent_records_skipped(self):
        assert list(_stream([{"owner": "acme"}]).stream_slices()) == []


class TestNextPageToken:
    def test_follows_page_info(self):
        stream = _stream()
        body = graphql_body(PR_PATH, [_pr_node()], {"hasNextPage": True, "endCursor": "e"})
        assert stream.next_page_token(FakeResponse(body)) == {"after": "e"}

    def test_early_exit_below_cursor(self):
        stream = _stream()
        stream._current_cursor_value = "2026-01-15T00:00:00Z"
        body = graphql_body(
            PR_PATH, [_pr_node(updated="2026-01-10T00:00:00Z")], {"hasNextPage": True, "endCursor": "e"}
        )
        assert stream.next_page_token(FakeResponse(body)) is None

    def test_early_exit_below_start_date(self):
        stream = _stream(start_date="2026-01-12")
        body = graphql_body(
            PR_PATH, [_pr_node(updated="2026-01-10T00:00:00Z")], {"hasNextPage": True, "endCursor": "e"}
        )
        assert stream.next_page_token(FakeResponse(body)) is None

    def test_no_more_pages(self):
        body = graphql_body(PR_PATH, [_pr_node()], {"hasNextPage": False, "endCursor": None})
        assert _stream().next_page_token(FakeResponse(body)) is None


class TestParseResponse:
    def _records(self, stream, nodes, stream_slice=SLICE):
        body = graphql_body(PR_PATH, nodes)
        return list(stream.parse_response(FakeResponse(body), stream_slice=stream_slice))

    def test_record_mapping(self):
        stream = _stream()
        rec = self._records(stream, [_pr_node()])[0]
        assert rec["unique_key"] == "T:S:acme:r1:1001"
        assert rec["database_id"] == 1001
        assert rec["state"] == "OPEN"
        assert rec["labels"] == ["bug"]
        assert rec["milestone_title"] == "v1"
        assert rec["merge_commit_sha"] == "mc1"
        assert rec["author_login"] == "al"
        assert rec["requested_reviewers"] == ["rev1"]
        assert rec["requested_teams"] == ["team-x"]
        assert rec["commit_count"] == 1 and rec["review_count"] == 2
        assert rec["repo_owner"] == "acme" and rec["repo_name"] == "r1"

    def test_state_normalization(self):
        stream = _stream()
        records = self._records(
            stream, [_pr_node(1, merged=True, state="MERGED"), _pr_node(2, state="CLOSED"), _pr_node(3, state="OPEN")]
        )
        assert [r["state"] for r in records] == ["MERGED", "CLOSED", "OPEN"]

    def test_records_at_or_below_cursor_skipped(self):
        stream = _stream()
        cursor_slice = {**SLICE, "cursor_value": "2026-01-10T00:00:00Z"}
        records = self._records(
            stream,
            [
                _pr_node(1, updated="2026-01-12T00:00:00Z"),
                _pr_node(2, updated="2026-01-10T00:00:00Z"),  # equal — already synced
            ],
            stream_slice=cursor_slice,
        )
        assert [r["number"] for r in records] == [1]

    def test_records_before_start_date_skipped(self):
        stream = _stream(start_date="2026-01-05")
        records = self._records(stream, [_pr_node(1, updated="2026-01-01T00:00:00Z")])
        assert records == []

    def test_child_slice_cache_populated(self):
        stream = _stream()
        self._records(stream, [_pr_node()])
        [child] = stream._child_slice_cache.values()
        assert child["number"] == 1
        assert child["commits_complete"] is True
        assert child["reviews_complete"] is False
        assert child["reviews_end_cursor"] == "rev-cur"
        assert child["embedded_offset"] == 0

    def test_errors_without_data_raise(self):
        stream = _stream()
        with pytest.raises(RuntimeError, match="GraphQL query failed"):
            list(stream.parse_response(FakeResponse({"errors": [{"message": "boom"}]}), stream_slice=SLICE))

    def test_partial_errors_freeze_partition(self):
        stream = _stream()
        body = graphql_body(PR_PATH, [_pr_node()])
        body["errors"] = [{"message": "partial"}]
        records = list(stream.parse_response(FakeResponse(body), stream_slice=SLICE))
        assert len(records) == 1
        assert "acme/r1" in stream._partitions_with_errors


class TestEmbeddedData:
    def test_round_trip_via_offsets(self):
        stream = _stream()
        body = graphql_body(PR_PATH, [_pr_node(1), _pr_node(2)])
        list(stream.parse_response(FakeResponse(body), stream_slice=SLICE))
        children = {c["number"]: c for c in stream._child_slice_cache.values()}
        commits_2 = stream.read_embedded_data(children[2]["embedded_offset"], "commits")
        assert commits_2["nodes"][0]["commit"]["oid"] == "c1"
        reviews_1 = stream.read_embedded_data(children[1]["embedded_offset"], "reviews")
        assert reviews_1["has_next_page"] is True

    def test_offset_past_eof_returns_empty(self):
        stream = _stream()
        body = graphql_body(PR_PATH, [_pr_node()])
        list(stream.parse_response(FakeResponse(body), stream_slice=SLICE))
        assert stream.read_embedded_data(10_000_000, "commits") == {}

    def test_corrupt_line_returns_empty(self):
        stream = _stream()
        stream._embedded_data_file.write("{not json\n")
        assert stream.read_embedded_data(0, "commits") == {}


class TestGetChildSlices:
    def test_returns_cache_when_built(self):
        stream = _stream()
        stream._child_cache_built = True
        stream._child_slice_cache[("acme", "r1", 1)] = {"number": 1}
        assert stream.get_child_slices() == [{"number": 1}]

    def test_fallback_triggers_read(self):
        # No parent repos → the fallback read completes without HTTP.
        stream = _stream([])
        assert stream.get_child_slices() == []
        assert stream._child_cache_built is True


class TestGetUpdatedState:
    def _record(self, updated="2026-01-10T00:00:00Z"):
        return {"repo_owner": "acme", "repo_name": "r1", "updated_at": updated}

    def test_cursor_advances(self):
        state = _stream().get_updated_state({}, self._record())
        assert state == {"acme/r1": {"updated_at": "2026-01-10T00:00:00Z"}}

    def test_older_record_does_not_regress(self):
        stream = _stream()
        state = {"acme/r1": {"updated_at": "2026-01-15T00:00:00Z"}}
        out = stream.get_updated_state(state, self._record("2026-01-10T00:00:00Z"))
        assert out["acme/r1"]["updated_at"] == "2026-01-15T00:00:00Z"

    def test_frozen_partition_untouched(self):
        stream = _stream()
        stream._partitions_with_errors.add("acme/r1")
        assert stream.get_updated_state({}, self._record()) == {}


class TestReadRecords:
    def test_none_slice_iterates_repos_and_marks_cache_built(self, monkeypatch):
        stream = _stream([REPO_RECORD])
        seen_slices = []

        def fake_read(self, sync_mode=None, stream_slice=None, stream_state=None, **kw):
            seen_slices.append(stream_slice)
            yield {"number": 1}

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        records = list(stream.read_records())
        assert len(records) == 1
        assert seen_slices[0]["owner"] == "acme"
        assert stream._child_cache_built is True

    def test_explicit_slice_passed_through(self, monkeypatch):
        stream = _stream()

        def fake_read(self, sync_mode=None, stream_slice=None, stream_state=None, **kw):
            yield {"slice": stream_slice}

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        records = list(stream.read_records(stream_slice=SLICE))
        assert records == [{"slice": SLICE}]
        assert stream._child_cache_built is False


class TestSchema:
    def test_schema_has_pr_fields(self, pull_requests_stream):
        props = pull_requests_stream.get_json_schema()["properties"]
        for field in ("number", "state", "labels", "requested_reviewers", "merged_at"):
            assert field in props
