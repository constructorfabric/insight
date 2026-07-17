from __future__ import annotations

from pathlib import Path

import pytest
from airbyte_cdk.sources.streams.http import HttpStream
from source_github_v2.streams.commits import CommitsStream
from tests.conftest import SHARED, FakeBranchParent, FakeResponse, graphql_body

HISTORY_PATH = ["repository", "ref", "target", "history"]


def _stream(branch_records=(), start_date=None, page_size=100) -> CommitsStream:
    return CommitsStream(parent=FakeBranchParent(branch_records), start_date=start_date, page_size=page_size, **SHARED)


def _branch(name="main", owner="acme", repo="r1", sha="abc", default_branch="main", pushed_at="2026-01-02T00:00:00Z"):
    return {
        "name": name,
        "repo_owner": owner,
        "repo_name": repo,
        "default_branch": default_branch,
        "pushed_at": pushed_at,
        "commit": {"sha": sha},
    }


def _slice(branch="main", owner="acme", repo="r1", **overrides):
    base = {
        "owner": owner,
        "repo": repo,
        "branch": branch,
        "default_branch": "main",
        "partition_key": f"{owner}/{repo}/{branch}",
        "cursor_value": None,
        "head_sha": "abc",
        "stop_at_sha": "",
        "repo_pushed_at": "2026-01-02T00:00:00Z",
        "_skipped_siblings": [],
    }
    base.update(overrides)
    return base


def _node(oid="c1", committed="2026-01-01T10:00:00Z", parents=("p1",)):
    return {
        "oid": oid,
        "message": f"msg {oid}",
        "committedDate": committed,
        "authoredDate": committed,
        "additions": 1,
        "deletions": 2,
        "changedFilesIfAvailable": 3,
        "author": {"name": "Al", "email": "al@x", "user": {"login": "al", "databaseId": 7}},
        "committer": {"name": "Bo", "email": "bo@x", "user": {"login": "bo", "databaseId": 8}},
        "parents": {"nodes": [{"oid": p} for p in parents]},
    }


class TestVariables:
    def test_full_slice(self):
        stream = _stream(page_size=50)
        variables = stream._variables(_slice())
        assert variables == {"owner": "acme", "repo": "r1", "branch": "refs/heads/main", "first": 50}

    def test_incomplete_slice_raises(self):
        with pytest.raises(ValueError, match="incomplete slice"):
            _stream()._variables({"owner": "acme", "repo": "r1"})

    def test_pagination_cursor_added(self):
        variables = _stream()._variables(_slice(), next_page_token={"after": "cur9"})
        assert variables["after"] == "cur9"

    def test_cursor_value_becomes_since(self):
        variables = _stream()._variables(_slice(cursor_value="2026-01-01T10:00:00Z"))
        assert variables["since"] == "2026-01-01T10:00:00Z"

    def test_bare_date_start_date_expanded(self):
        variables = _stream(start_date="2026-01-01")._variables(_slice())
        assert variables["since"] == "2026-01-01T00:00:00Z"


class TestExtractors:
    def test_nodes_and_page_info(self):
        body = graphql_body(HISTORY_PATH, [_node()], {"hasNextPage": True, "endCursor": "e"})
        stream = _stream()
        assert stream._extract_nodes(body["data"]) == [_node()]
        assert stream._extract_page_info(body["data"]) == {"hasNextPage": True, "endCursor": "e"}

    def test_missing_ref_tolerated(self):
        # Deleted branch: ref resolves to null in GraphQL.
        stream = _stream()
        assert stream._extract_nodes({"repository": {"ref": None}}) == []
        assert stream._extract_page_info({"repository": {"ref": None}}) == {}


class TestNextPageToken:
    def test_follows_page_info(self):
        body = graphql_body(HISTORY_PATH, [], {"hasNextPage": True, "endCursor": "e2"})
        assert _stream().next_page_token(FakeResponse(body)) == {"after": "e2"}

    def test_stop_pagination_flag_consumed(self):
        stream = _stream()
        stream._stop_pagination = True
        body = graphql_body(HISTORY_PATH, [], {"hasNextPage": True, "endCursor": "e"})
        assert stream.next_page_token(FakeResponse(body)) is None
        assert stream._stop_pagination is False  # one-shot

    def test_stop_at_sha_in_page_ends_pagination(self):
        stream = _stream()
        stream._current_stop_at_sha = "known"
        body = graphql_body(HISTORY_PATH, [_node("known")], {"hasNextPage": True, "endCursor": "e"})
        assert stream.next_page_token(FakeResponse(body)) is None

    def test_stop_at_sha_absent_continues(self):
        stream = _stream()
        stream._current_stop_at_sha = "known"
        body = graphql_body(HISTORY_PATH, [_node("other")], {"hasNextPage": True, "endCursor": "e"})
        assert stream.next_page_token(FakeResponse(body)) == {"after": "e"}


class TestStreamSlices:
    def test_basic_slice_fields(self):
        stream = _stream([_branch()])
        slices = list(stream.stream_slices())
        assert slices == [
            {
                "owner": "acme",
                "repo": "r1",
                "branch": "main",
                "default_branch": "main",
                "partition_key": "acme/r1/main",
                "cursor_value": None,
                "head_sha": "abc",
                "stop_at_sha": "",
                "repo_pushed_at": "2026-01-02T00:00:00Z",
                "_skipped_siblings": [],
            }
        ]

    def test_repo_freshness_gate_skips_unchanged_repo(self):
        stream = _stream([_branch()])
        state = {"_repo:acme/r1": {"pushed_at": "2026-01-02T00:00:00Z"}}
        assert list(stream.stream_slices(stream_state=state)) == []

    def test_repo_freshness_gate_passes_newer_push(self):
        stream = _stream([_branch(pushed_at="2026-01-03T00:00:00Z")])
        state = {"_repo:acme/r1": {"pushed_at": "2026-01-02T00:00:00Z"}}
        assert len(list(stream.stream_slices(stream_state=state))) == 1

    def test_duplicate_head_sha_branches_deduped(self):
        # main and mirror share a HEAD; main wins (default branch sorts first)
        stream = _stream([_branch("mirror", sha="abc"), _branch("main", sha="abc")])
        slices = list(stream.stream_slices())
        assert [s["branch"] for s in slices] == ["main"]
        assert slices[0]["_skipped_siblings"] == ["acme/r1/mirror"]

    def test_branch_without_head_still_selected(self):
        record = _branch("ghost")
        record["commit"] = None
        slices = list(_stream([record]).stream_slices())
        assert [s["branch"] for s in slices] == ["ghost"]

    def test_head_unchanged_branch_skipped(self):
        stream = _stream([_branch()])
        state = {"acme/r1/main": {"head_sha": "abc"}}
        assert list(stream.stream_slices(stream_state=state)) == []

    def test_force_push_resets_cursor(self):
        stream = _stream([_branch(sha="new")])
        state = {"acme/r1/main": {"head_sha": "old", "committed_date": "2026-01-01T00:00:00Z"}}
        slices = list(stream.stream_slices(stream_state=state))
        assert slices[0]["cursor_value"] is None
        assert slices[0]["stop_at_sha"] == "old"

    def test_unchanged_head_keeps_cursor(self):
        stream = _stream([_branch()])
        state = {"acme/r1/main": {"committed_date": "2026-01-01T00:00:00Z"}}
        slices = list(stream.stream_slices(stream_state=state))
        assert slices[0]["cursor_value"] == "2026-01-01T00:00:00Z"

    def test_seen_hash_skip_defers_state_update(self):
        # Interleave like the CDK: after the default branch is read, sibling
        # branches whose HEAD already appeared in main's history are skipped.
        stream = _stream([_branch("main", sha="abc"), _branch("feat", sha="feat-head")])
        slice_iter = stream.stream_slices()
        first = next(slice_iter)
        assert first["branch"] == "main"
        stream._seen_hashes["feat-head"] = "acme/r1"  # as parse_response would
        assert list(slice_iter) == []
        assert stream._deferred_state_updates == {"acme/r1/feat": {"head_sha": "feat-head"}}

    def test_incomplete_parent_records_ignored(self):
        assert list(_stream([{"repo_owner": "acme"}]).stream_slices()) == []


class TestParseResponse:
    def test_record_mapping(self):
        stream = _stream()
        body = graphql_body(HISTORY_PATH, [_node("c1")])
        records = list(stream.parse_response(FakeResponse(body), stream_slice=_slice()))
        assert len(records) == 1
        rec = records[0]
        assert rec["unique_key"] == "T:S:acme:r1:c1"
        assert rec["sha"] == "c1"
        assert rec["author_login"] == "al"
        assert rec["committer_id"] == 8
        assert rec["parent_hashes"] == ["p1"]
        assert rec["branch_name"] == "main"
        assert rec["head_sha"] == "abc"
        assert rec["data_source"] == "insight_github"

    def test_null_author_tolerated(self):
        stream = _stream()
        node = _node("c1")
        node["author"] = None
        node["committer"] = {"name": "Bo", "email": "bo@x", "user": None}
        body = graphql_body(HISTORY_PATH, [node])
        rec = list(stream.parse_response(FakeResponse(body), stream_slice=_slice()))[0]
        assert rec["author_login"] is None
        assert rec["committer_login"] is None

    def test_stop_at_sha_ends_branch(self):
        stream = _stream()
        body = graphql_body(HISTORY_PATH, [_node("new"), _node("known"), _node("older")])
        records = list(stream.parse_response(FakeResponse(body), stream_slice=_slice(stop_at_sha="known")))
        assert [r["sha"] for r in records] == ["new"]
        assert stream._stop_pagination is True

    def test_seen_commit_dedup_stops_pagination(self):
        stream = _stream()
        stream._seen_hashes["shared"] = "acme/r1"
        body = graphql_body(HISTORY_PATH, [_node("new"), _node("shared")])
        records = list(stream.parse_response(FakeResponse(body), stream_slice=_slice("feat")))
        assert [r["sha"] for r in records] == ["new"]
        assert stream._stop_pagination is True

    def test_errors_without_data_raise(self):
        stream = _stream()
        with pytest.raises(RuntimeError, match="GraphQL query failed"):
            list(stream.parse_response(FakeResponse({"errors": [{"message": "boom"}]}), stream_slice=_slice()))

    def test_partial_errors_freeze_partition(self):
        stream = _stream()
        body = graphql_body(HISTORY_PATH, [_node("c1")])
        body["errors"] = [{"message": "partial"}]
        records = list(stream.parse_response(FakeResponse(body), stream_slice=_slice()))
        assert len(records) == 1
        assert "acme/r1/main" in stream._partitions_with_errors

    def test_error_flag_cleared_on_next_partition(self):
        stream = _stream()
        body = graphql_body(HISTORY_PATH, [_node("c1")])
        body["errors"] = [{"message": "partial"}]
        list(stream.parse_response(FakeResponse(body), stream_slice=_slice("main")))
        clean = graphql_body(HISTORY_PATH, [_node("c2")])
        list(stream.parse_response(FakeResponse(clean), stream_slice=_slice("feat")))
        assert "acme/r1/main" not in stream._partitions_with_errors

    def test_commit_meta_written_for_file_changes(self):
        stream = _stream()
        body = graphql_body(HISTORY_PATH, [_node("c1", parents=("p1", "p2"))])
        list(stream.parse_response(FakeResponse(body), stream_slice=_slice()))
        path = Path(stream.get_commit_meta_path())
        assert path.read_text() == "c1\tacme\tr1\t2026-01-01T10:00:00Z\t2\n"


class TestGetUpdatedState:
    def _record(self, committed="2026-01-05T00:00:00Z", branch="main", head="abc"):
        return {
            "repo_owner": "acme",
            "repo_name": "r1",
            "branch_name": branch,
            "committed_date": committed,
            "head_sha": head,
            "repo_pushed_at": "2026-01-06T00:00:00Z",
        }

    def test_cursor_and_head_advance(self):
        stream = _stream()
        state = stream.get_updated_state({}, self._record())
        assert state["acme/r1/main"] == {"committed_date": "2026-01-05T00:00:00Z", "head_sha": "abc"}
        assert state["_repo:acme/r1"] == {"pushed_at": "2026-01-06T00:00:00Z"}

    def test_older_record_does_not_regress_cursor(self):
        stream = _stream()
        state = {"acme/r1/main": {"committed_date": "2026-01-09T00:00:00Z"}}
        out = stream.get_updated_state(state, self._record("2026-01-05T00:00:00Z"))
        assert out["acme/r1/main"]["committed_date"] == "2026-01-09T00:00:00Z"

    def test_frozen_partition_untouched(self):
        stream = _stream()
        stream._partitions_with_errors.add("acme/r1/main")
        assert stream.get_updated_state({}, self._record()) == {}

    def test_skipped_siblings_mirror_cursor(self):
        stream = _stream()
        stream._current_skipped_siblings = ["acme/r1/mirror"]
        state = stream.get_updated_state({}, self._record())
        assert state["acme/r1/mirror"] == state["acme/r1/main"]

    def test_deferred_updates_applied(self):
        stream = _stream()
        stream._deferred_state_updates = {
            "acme/r1/feat": {"head_sha": "feat-head"},
            "acme/r1/main": {"head_sha": "override"},
        }
        state = {"acme/r1/main": {"committed_date": "2026-01-01T00:00:00Z"}}
        out = stream.get_updated_state(state, self._record())
        assert out["acme/r1/feat"] == {"head_sha": "feat-head"}
        # Existing entries are merged, not replaced
        assert out["acme/r1/main"]["committed_date"] == "2026-01-05T00:00:00Z"
        assert out["acme/r1/main"]["head_sha"] == "override"
        assert stream._deferred_state_updates == {}


class TestReadRecords:
    def test_none_slice_iterates_stream_slices(self, monkeypatch):
        stream = _stream([_branch()])
        seen_slices = []

        def fake_read(self, sync_mode=None, stream_slice=None, stream_state=None, **kw):
            seen_slices.append(stream_slice)
            yield {"sha": "c1"}

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        records = list(stream.read_records())
        assert len(records) == 1
        assert seen_slices[0]["branch"] == "main"

    def test_explicit_slice_passed_through(self, monkeypatch):
        stream = _stream()
        seen_slices = []

        def fake_read(self, sync_mode=None, stream_slice=None, stream_state=None, **kw):
            seen_slices.append(stream_slice)
            yield {"sha": "c1"}

        monkeypatch.setattr(HttpStream, "read_records", fake_read)
        list(stream.read_records(stream_slice=_slice("feat")))
        assert seen_slices == [_slice("feat")]


class TestSchemaAndQuery:
    def test_query_is_bulk_commit_query(self, commits_stream):
        assert "history(first: $first" in commits_stream._query()

    def test_schema_has_commit_fields(self, commits_stream):
        props = commits_stream.get_json_schema()["properties"]
        for field in ("sha", "parent_hashes", "branch_name", "committed_date"):
            assert field in props
